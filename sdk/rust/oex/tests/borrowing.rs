#[test]
fn error_and_type_conversions_preserve_semantics() {
    let error = oex::Error::new("Plugin:Domain", "invalid argument");
    assert_eq!(error.kind(), oex::ErrorKind::Plugin);
    assert_eq!(error.identifier(), Some("Plugin:Domain"));
    assert_eq!(oex::ErrorKind::from_status(71).status(), 71);
    assert_ne!(oex::ErrorKind::Unknown(0).status(), 0);
    assert_eq!(oex::DataType::from_raw(71), oex::DataType::Unknown(71));
    assert_eq!(oex::ValueKind::from_raw(71), oex::ValueKind::Unknown(71));
    assert!(oex::Logical::from(true).get());
    assert_eq!(oex::Char16(0xd800).0, 0xd800);
}
