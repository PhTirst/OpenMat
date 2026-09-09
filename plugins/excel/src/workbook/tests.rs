use super::*;
use rust_xlsxwriter::{Format, Formula};
use std::io::Write;

fn options() -> WriteOptions<'static> {
    WriteOptions {
        sheet: "测量 Δ",
        start: (2, 1),
        overwrite: false,
    }
}

#[test]
fn numeric_round_trip_layout_unicode_origin_and_blank_padding() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("实验 数据.xlsx");
    let data = [1.0, 2.0, 3.0, 4.0, f64::NAN, 6.0];
    write(&path, &[2, 3], &data, &options(), || false).unwrap();
    let names = sheet_names(&path).unwrap();
    assert_eq!(names, ["测量 Δ"]);
    let m = read(&path, &Sheet::Name(names[0].clone()), None, true, || false).unwrap();
    assert_eq!((m.rows, m.cols, m.origin), (2, 3, (2, 1)));
    assert_eq!(&m.values[..4], &data[..4]);
    assert!(m.values[4].is_nan());
    assert_eq!(m.values[5], 6.0);
    assert_eq!(m.kinds, [1, 1, 1, 1, 0, 1]);
    let padded = read(
        &path,
        &Sheet::Index(0),
        Some(Rect::parse("A1:E5").unwrap()),
        false,
        || false,
    )
    .unwrap();
    assert_eq!((padded.rows, padded.cols, padded.origin), (5, 5, (0, 0)));
    assert_eq!(padded.values[7], 1.0);
    assert!(padded.values[0].is_nan());
    assert!(padded.values[24].is_nan());
    assert!(padded.kinds.is_empty());
}

#[test]
fn mixed_types_and_cached_formulas_are_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mixed.xlsx");
    let mut book = Workbook::new();
    book.add_worksheet().set_name("Empty").unwrap();
    let sheet = book.add_worksheet();
    sheet.set_name("Mixed").unwrap();
    sheet.write_number(1, 1, 42.0).unwrap();
    sheet.write_string(1, 2, "123").unwrap();
    sheet.write_boolean(1, 3, true).unwrap();
    sheet
        .write_number_with_format(1, 4, 45000.0, &Format::new().set_num_format("yyyy-mm-dd"))
        .unwrap();
    sheet
        .write_formula(1, 5, Formula::new("20+22").set_result("42"))
        .unwrap();
    sheet
        .write_formula(1, 6, Formula::new("1/0").set_result("#DIV/0!"))
        .unwrap();
    book.save(&path).unwrap();
    let m = read(
        &path,
        &Sheet::Index(1),
        Some(Rect::parse("A2:G2").unwrap()),
        true,
        || false,
    )
    .unwrap();
    assert_eq!(m.kinds, [0, 1, 3, 2, 4, 1, 5]);
    assert_eq!(m.values[1], 42.0);
    assert_eq!(m.values[3], 1.0);
    assert_eq!(m.values[5], 42.0);
    assert!(m.values[2].is_nan() && m.values[4].is_nan());
    assert!(m.values[6].is_nan());
    assert_eq!(sheet_names(&path).unwrap(), ["Empty", "Mixed"]);
    assert_eq!(
        read(&path, &Sheet::Index(0), None, false, || false)
            .unwrap()
            .values
            .len(),
        0
    );
    for selector in [Sheet::Index(2), Sheet::Name("missing".into())] {
        assert_eq!(
            read(&path, &selector, None, false, || false)
                .err()
                .unwrap()
                .code,
            "Sheet"
        );
    }
}

#[test]
fn read_authored_ods_fixture() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("numbers.ods");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    let opts =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("mimetype", opts).unwrap();
    zip.write_all(b"application/vnd.oasis.opendocument.spreadsheet")
        .unwrap();
    zip.start_file("META-INF/manifest.xml", opts).unwrap();
    zip.write_all(br#"<?xml version="1.0" encoding="UTF-8"?><manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/></manifest:manifest>"#).unwrap();
    zip.start_file("content.xml", opts).unwrap();
    let content = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" office:version="1.2">
<office:body><office:spreadsheet><table:table table:name="Data"><table:table-row>
<table:table-cell office:value-type="float" office:value="7.5"><text:p>7.5</text:p></table:table-cell>
<table:table-cell office:value-type="boolean" office:boolean-value="true"><text:p>TRUE</text:p></table:table-cell>
<table:table-cell office:value-type="string"><text:p>header</text:p></table:table-cell>
</table:table-row></table:table></office:spreadsheet></office:body></office:document-content>"#;
    zip.write_all(content.replace(['\r', '\n'], "").as_bytes())
        .unwrap();
    zip.finish().unwrap();
    let m = read(&path, &Sheet::Index(0), None, true, || false).unwrap();
    assert_eq!((m.rows, m.cols), (1, 3));
    assert_eq!(m.kinds, [1, 2, 3]);
    assert_eq!(&m.values[..2], &[7.5, 1.0]);
}

#[test]
fn logical_integer_and_missing_values() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("logical.xlsx");
    write(
        &path,
        &[1, 2],
        &[Logical::from(true), Logical::from(false)],
        &options(),
        || false,
    )
    .unwrap();
    let m = read(&path, &Sheet::Index(0), None, true, || false).unwrap();
    assert_eq!(m.values, [1.0, 0.0]);
    assert_eq!(m.kinds, [2, 2]);
    let path = dir.path().join("integer.xlsx");
    write(&path, &[1, 2], &[-8_i64, 100], &options(), || false).unwrap();
    assert_eq!(
        read(&path, &Sheet::Index(0), None, false, || false)
            .unwrap()
            .values,
        [-8.0, 100.0]
    );
    let path = dir.path().join("blank.xlsx");
    write(&path, &[2, 2], &[f64::NAN; 4], &options(), || false).unwrap();
    let m = read(
        &path,
        &Sheet::Index(0),
        Some(Rect::parse("B3:C4").unwrap()),
        true,
        || false,
    )
    .unwrap();
    assert_eq!(m.kinds, [0; 4]);
    assert!(m.values.iter().all(|v| v.is_nan()));
}

#[test]
fn failed_and_cancelled_writes_preserve_destination() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("existing.xlsx");
    let original = b"existing user content";
    std::fs::write(&path, original).unwrap();
    assert_eq!(
        write(&path, &[1, 1], &[1.0], &options(), || false)
            .err()
            .unwrap()
            .code,
        "Exists"
    );
    let overwrite = WriteOptions {
        overwrite: true,
        ..options()
    };
    for bad in [f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(
            write(&path, &[1, 1], &[bad], &overwrite, || false)
                .err()
                .unwrap()
                .code,
            "NonFinite"
        );
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
    assert_eq!(
        write(&path, &[1, 1], &[u64::MAX], &overwrite, || false)
            .err()
            .unwrap()
            .code,
        "Precision"
    );
    let polls = std::cell::Cell::new(0);
    assert_eq!(
        write(&path, &[5000, 1], &[0.0; 5000], &overwrite, || {
            polls.set(polls.get() + 1);
            polls.get() > 2
        })
        .err()
        .unwrap()
        .code,
        "Cancelled"
    );
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    write(&path, &[1, 1], &[8.0], &overwrite, || false).unwrap();
    assert_eq!(
        read(&path, &Sheet::Index(0), None, false, || false)
            .unwrap()
            .values,
        [8.0]
    );
}

#[test]
fn reject_invalid_dimensions_limits_paths_and_workbooks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.xlsx");
    for dims in [
        &[0, 0][..],
        &[1, 1, 2],
        &[ROWS as u64 + 1, 1],
        &[1, COLS as u64 + 1],
    ] {
        assert_eq!(
            write(&path, dims, &[1.0], &options(), || false)
                .err()
                .unwrap()
                .code,
            "Size"
        );
    }
    assert_eq!(
        write(
            &path.with_extension("xls"),
            &[1, 1],
            &[1.0],
            &options(),
            || false
        )
        .err()
        .unwrap()
        .code,
        "Format"
    );
    let bad_name = WriteOptions {
        sheet: "bad/name",
        ..options()
    };
    assert_eq!(
        write(&path, &[1, 1], &[1.0], &bad_name, || false)
            .err()
            .unwrap()
            .code,
        "Write"
    );
    assert!(!path.exists());
    assert!(read(&path, &Sheet::Index(0), None, false, || false).is_err());
    write(&path, &[1, 1], &[1.0], &options(), || false).unwrap();
    assert_eq!(
        read(
            &path,
            &Sheet::Index(0),
            Some(Rect::parse("A1:XFD1048576").unwrap()),
            false,
            || false
        )
        .err()
        .unwrap()
        .code,
        "Size"
    );
    assert_eq!(
        read(&path, &Sheet::Index(0), None, false, || true)
            .err()
            .unwrap()
            .code,
        "Cancelled"
    );
    std::fs::write(&path, b"not an xlsx").unwrap();
    assert_eq!(
        read(&path, &Sheet::Index(0), None, false, || false)
            .err()
            .unwrap()
            .code,
        "IO"
    );
}
