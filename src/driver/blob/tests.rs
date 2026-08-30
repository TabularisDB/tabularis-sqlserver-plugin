use super::*;

#[test]
fn wire_format_contains_size_mime_and_base64() {
    let bytes = [0xCA, 0xFE, 0xBA, 0xBE];
    assert_eq!(
        encode_blob_full(&bytes, 4).unwrap(),
        "BLOB:4:application/octet-stream:yv66vg=="
    );
}

#[test]
fn wire_format_sniffs_png_magic_bytes() {
    let png_signature = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let wire = encode_blob_full(&png_signature, 8).unwrap();
    assert!(wire.starts_with("BLOB:8:image/png:"));
}

#[test]
fn preview_size_ceiling_accepts_exact_limit_and_rejects_larger_value() {
    assert!(encode_blob_full(&[1, 2, 3, 4], 4).is_ok());

    let error = encode_blob_full(&[1, 2, 3, 4], 3).unwrap_err();
    assert!(error.contains("4 bytes"));
    assert!(error.contains("max_blob_size of 3 bytes"));
}

#[test]
fn binary_varbinary_and_image_are_user_blob_types() {
    for data_type in ["binary(8)", "VARBINARY(MAX)", "image"] {
        assert!(
            validate_blob_data_type(data_type).is_ok(),
            "rejected {data_type}"
        );
    }
}

#[test]
fn rowversion_and_timestamp_are_not_offered_as_blobs() {
    for data_type in ["ROWVERSION", "timestamp"] {
        let error = validate_blob_data_type(data_type).unwrap_err();
        assert!(error.contains("concurrency token"));
    }
}

#[test]
fn writable_path_validation_rejects_empty_directory_and_missing_parent() {
    assert!(validate_writable_file_path("").is_err());
    assert!(validate_writable_file_path("/tmp").is_err());
    assert!(validate_writable_file_path("/this-directory-must-not-exist-ss012/out.bin").is_err());
}

#[test]
fn writable_path_validation_accepts_existing_parent_and_bare_filename() {
    assert!(validate_writable_file_path("/tmp/ss012-output.bin").is_ok());
    assert!(validate_writable_file_path("ss012-output.bin").is_ok());
}
