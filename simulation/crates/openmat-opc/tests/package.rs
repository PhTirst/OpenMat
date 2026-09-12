use std::io::{Cursor, Write};

use openmat_opc::{Limits, Package, parse_xml, resolve_target};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

const TYPES: &str = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="json" ContentType="application/json"/></Types>"#;
const ROOT: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="document" Type="urn:openmat:test" Target="model/main.xml"/></Relationships>"#;

fn pack(entries: &[(&str, &str)], method: CompressionMethod) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, contents) in entries {
        writer
            .start_file(
                *name,
                SimpleFileOptions::default().compression_method(method),
            )
            .unwrap();
        writer.write_all(contents.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}
fn basic(extra: &[(&str, &str)], method: CompressionMethod) -> Vec<u8> {
    let mut entries = vec![
        ("[Content_Types].xml", TYPES),
        ("_rels/.rels", ROOT),
        ("model/main.xml", "<Model/>"),
    ];
    entries.extend(extra);
    pack(&entries, method)
}

#[test]
fn reads_stored_and_deflated_parts_and_relationships_without_extraction() {
    for method in [CompressionMethod::Stored, CompressionMethod::Deflated] {
        let bytes = basic(&[("assets/data.json", "{\"value\":3}")], method);
        let package = Package::read(&bytes, Limits::default()).unwrap();
        assert_eq!(package.relationships("/")[0].target, "/model/main.xml");
        assert_eq!(
            package.part("/assets/data.json").unwrap().content_type(),
            "application/json"
        );
        assert!(
            package
                .xml("/MODEL/Main.xml")
                .unwrap()
                .root_element()
                .has_tag_name("Model")
        );
    }
}

#[test]
fn relative_uris_and_external_relationships_have_distinct_behavior() {
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="a" Type="urn:test" Target="../assets/data.json"/><Relationship Id="b" Type="urn:test" Target="https://example.invalid/file" TargetMode="External"/></Relationships>"#;
    let bytes = basic(
        &[
            ("model/_rels/main.xml.rels", rels),
            ("assets/data.json", "{}"),
        ],
        CompressionMethod::Deflated,
    );
    let package = Package::read(&bytes, Limits::default()).unwrap();
    let rels = package.relationships("/model/main.xml");
    assert_eq!(rels[0].target, "/assets/data.json");
    assert!(!rels[0].external);
    assert!(rels[1].external);
    assert_eq!(rels[1].target, "https://example.invalid/file");
    assert_eq!(resolve_target("/a/b.xml", "../c.xml").unwrap(), "/c.xml");
    for target in [
        "../../escape.xml",
        "../%2e%2e/file",
        "C:/file",
        "//host/file",
        "x\\y",
        "x%2fy",
    ] {
        assert!(resolve_target("/a/b.xml", target).is_err(), "{target}");
    }
}

#[test]
fn rejects_ambiguous_parts_missing_targets_and_invalid_names() {
    for (entry, code) in [
        ("model/MAIN.xml", "duplicate_part"),
        ("../escape.xml", "part_name"),
        ("model/%6dain.xml", "part_name"),
    ] {
        let bytes = basic(&[(entry, "<Other/>")], CompressionMethod::Stored);
        assert_eq!(
            Package::read(&bytes, Limits::default()).unwrap_err().code,
            code
        );
    }
    let bytes = pack(
        &[("[Content_Types].xml", TYPES), ("_rels/.rels", ROOT)],
        CompressionMethod::Stored,
    );
    assert_eq!(
        Package::read(&bytes, Limits::default()).unwrap_err().code,
        "missing_target"
    );
}

#[test]
fn content_type_overrides_and_namespaces_are_validated() {
    let types = TYPES.replace(
        "</Types>",
        "<Override PartName=\"/model/main.xml\" ContentType=\"application/vnd.test+xml\"/></Types>",
    );
    let bytes = pack(
        &[
            ("[Content_Types].xml", &types),
            ("model/main.xml", "<Model/>"),
        ],
        CompressionMethod::Stored,
    );
    assert_eq!(
        Package::read(&bytes, Limits::default())
            .unwrap()
            .part("/model/main.xml")
            .unwrap()
            .content_type(),
        "application/vnd.test+xml"
    );
    let wrong = TYPES.replace(
        "http://schemas.openxmlformats.org/package/2006/content-types",
        "urn:wrong",
    );
    let bytes = pack(
        &[("[Content_Types].xml", &wrong)],
        CompressionMethod::Stored,
    );
    assert_eq!(
        Package::read(&bytes, Limits::default()).unwrap_err().code,
        "content_types"
    );
}

#[test]
fn bounded_reading_rejects_archive_parts_expansion_and_xml_entities() {
    let bytes = basic(&[], CompressionMethod::Deflated);
    for limits in [
        Limits {
            archive_bytes: 10,
            ..Limits::default()
        },
        Limits {
            parts: 1,
            ..Limits::default()
        },
        Limits {
            part_bytes: 10,
            ..Limits::default()
        },
        Limits {
            total_bytes: 10,
            ..Limits::default()
        },
    ] {
        assert!(Package::read(&bytes, limits).is_err());
    }
    assert!(
        parse_xml(
            b"<!DOCTYPE test [<!ENTITY x SYSTEM 'file:///unused'>]><test>&x;</test>",
            Limits::default()
        )
        .is_err()
    );
    assert!(
        parse_xml(
            b"<x><y/><z/></x>",
            Limits {
                xml_nodes: 2,
                ..Limits::default()
            }
        )
        .is_err()
    );
    assert!(Package::read(b"not a package", Limits::default()).is_err());
}

#[test]
fn corrupted_part_crc_is_rejected() {
    let mut bytes = basic(&[], CompressionMethod::Stored);
    let index = bytes.windows(8).position(|v| v == b"<Model/>").unwrap();
    bytes[index + 1] = b'X';
    assert_eq!(
        Package::read(&bytes, Limits::default()).unwrap_err().code,
        "zip_data"
    );
}

#[test]
fn rejects_ambiguous_footers_and_metadata_allocation_before_reading_entries() {
    let original = basic(&[], CompressionMethod::Stored);
    let end = original.len() - 22;
    let mut many = original.clone();
    many[end + 8..end + 12].copy_from_slice(&[0xfe, 0xff, 0xfe, 0xff]);
    assert_eq!(
        Package::read(&many, Limits::default()).unwrap_err().code,
        "part_limit"
    );
    let mut zip64 = original.clone();
    zip64[end + 8..end + 12].copy_from_slice(&[0xff; 4]);
    assert_eq!(
        Package::read(&zip64, Limits::default()).unwrap_err().code,
        "zip_profile"
    );
    let mut second = original.clone();
    second.extend_from_slice(&original[end..]);
    assert_eq!(
        Package::read(&second, Limits::default()).unwrap_err().code,
        "zip_profile"
    );
    let mut wrong_extent = original;
    wrong_extent[end + 12..end + 16].copy_from_slice(&0_u32.to_le_bytes());
    assert_eq!(
        Package::read(&wrong_extent, Limits::default())
            .unwrap_err()
            .code,
        "zip_profile"
    );
}

#[test]
fn duplicate_relationship_ids_and_missing_sources_are_rejected() {
    let duplicate = ROOT.replace(
        "</Relationships>",
        r#"<Relationship Id="document" Type="urn:test" Target="model/main.xml"/></Relationships>"#,
    );
    let bytes = pack(
        &[
            ("[Content_Types].xml", TYPES),
            ("_rels/.rels", &duplicate),
            ("model/main.xml", "<Model/>"),
        ],
        CompressionMethod::Stored,
    );
    assert_eq!(
        Package::read(&bytes, Limits::default()).unwrap_err().code,
        "duplicate_relationship"
    );
    let bytes = basic(
        &[(
            "absent/_rels/source.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#,
        )],
        CompressionMethod::Stored,
    );
    assert_eq!(
        Package::read(&bytes, Limits::default()).unwrap_err().code,
        "relationship_source"
    );
}
