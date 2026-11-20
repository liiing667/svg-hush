use svg_hush::*;

#[test]
fn duplicate_ids_are_rejected_and_fail_closed() {
    // Fixed two-element fixture: two <linearGradient> elements share id="shared",
    // and both a paint reference (fill="url(#shared)") and a <use href="#shared">
    // bind to that id. Browsers resolve such references to the first matching
    // element, which may differ from the element the references were audited
    // against once filtering drops elements. The whole document must be refused.
    let fixture = std::fs::read("tests/duplicate_ids.xml").unwrap();
    let mut f = Filter::new();
    f.set_data_url_filter(data_url_filter::allow_standard_images);
    let mut out = Vec::new();
    let err = f
        .filter(&mut fixture.as_slice(), &mut out)
        .expect_err("duplicate ids must be rejected");
    assert!(
        matches!(err, FError::DuplicateId(ref id) if id == "shared"),
        "unexpected error: {err:?}"
    );
    // Fail closed: nothing partial may reach the destination.
    assert!(
        out.is_empty(),
        "partial output leaked on rejection: {out:?}"
    );
}

#[test]
fn unique_ids_keep_legitimate_references() {
    // The same fixed fixture, but with the second element's id made unique.
    // Both references must be preserved (relative same-origin fragment only).
    let fixture = std::fs::read_to_string("tests/duplicate_ids.xml")
        .unwrap()
        .replacen(
            r#"<linearGradient id="shared" x1="0" y1="0" x2="0" y2="1">"#,
            r#"<linearGradient id="shared-2" x1="0" y1="0" x2="0" y2="1">"#,
            1,
        );
    let mut f = Filter::new();
    f.set_data_url_filter(data_url_filter::allow_standard_images);
    let mut out = Vec::new();
    f.filter(&mut fixture.as_bytes(), &mut out).unwrap();
    let s = std::str::from_utf8(&out).unwrap();
    assert!(s.contains(r#"id="shared""#), "first id kept: {s}");
    assert!(s.contains(r#"id="shared-2""#), "unique second id kept: {s}");
    assert!(
        s.contains(r#"fill="url(#shared)""#),
        "paint reference kept: {s}"
    );
    assert!(s.contains(r##"href="#shared""##), "use reference kept: {s}");
}

#[test]
fn unique_id_output_survives_refiltering() {
    // Re-filtering the accepted (unique-id) output must be idempotent and keep
    // the references intact, proving the emitted document has unique ids.
    let fixture = std::fs::read_to_string("tests/duplicate_ids.xml")
        .unwrap()
        .replacen(
            r#"<linearGradient id="shared" x1="0" y1="0" x2="0" y2="1">"#,
            r#"<linearGradient id="shared-2" x1="0" y1="0" x2="0" y2="1">"#,
            1,
        );
    let mut f = Filter::new();
    f.set_data_url_filter(data_url_filter::allow_standard_images);
    let mut out = Vec::new();
    f.filter(&mut fixture.as_bytes(), &mut out).unwrap();
    let mut out2 = Vec::new();
    f.filter(&mut out.as_slice(), &mut out2)
        .expect("re-filtering unique-id output must succeed");
    assert_eq!(out, out2, "filtering must be idempotent");
    let s = std::str::from_utf8(&out2).unwrap();
    assert!(s.contains(r#"fill="url(#shared)""#));
    assert!(s.contains(r##"href="#shared""##));
}

#[test]
fn duplicate_ids_detected_after_value_normalization() {
    // The id attribute is ASCII/whitespace-normalized before emission, so inputs
    // that normalize to the same output id must also be treated as duplicates.
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg">
        <rect id="dup"/>
        <rect id=" dup "/>
    </svg>"#;
    let mut out = Vec::new();
    let err = Filter::new()
        .filter(&mut svg.as_bytes(), &mut out)
        .expect_err("normalization-induced duplicate must be rejected");
    assert!(
        matches!(err, FError::DuplicateId(ref id) if id == "dup"),
        "unexpected: {err:?}"
    );
    assert!(out.is_empty());
}

#[test]
fn ids_of_dropped_elements_are_not_counted() {
    // A duplicate-looking id on a dropped (disallowed) element must not reject
    // the document, since that element and its id never reach the output.
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg">
        <script id="x">alert(1)</script>
        <rect id="x"/>
        <rect id="y"/>
    </svg>"#;
    let mut out = Vec::new();
    Filter::new()
        .filter(&mut svg.as_bytes(), &mut out)
        .expect("unique emitted ids must pass");
    let s = std::str::from_utf8(&out).unwrap();
    assert!(s.contains(r#"id="x""#));
    assert!(s.contains(r#"id="y""#));
}

#[test]
fn whole_file() {
    let test = std::fs::read("tests/test.xml").unwrap();
    let expected = std::fs::read_to_string("tests/filtered.xml").unwrap();
    let mut f = Filter::new();
    f.set_data_url_filter(data_url_filter::allow_standard_images);
    let mut out = Vec::new();
    f.filter(&mut test.as_slice(), &mut out).unwrap();
    // cargo run -- tests/test.xml  > tests/filtered.xml
    assert_eq!(std::str::from_utf8(&out).unwrap(), expected);
}

#[test]
fn ns() {
    let svg = r##"<?xml version="1.0" encoding="UTF-8" standalone="no"?>
    <svg xmlns="http://www.w3.org/2000/svg" xmlns:svg="http://www.w3.org/2000/svg" xmlns:vector="http://www.w3.org/2000/svg">
        <rect height="300" width="300"/>
        <svg:rect height="200" width="200">
            <title>Test</title>
        </svg:rect>
        <vector:rect height="100" width="100"/>
        <svg:text xml:space="preserve">  Hallo World  </svg:text>
    </svg>
    "##;

    let f = Filter::new();
    let mut out = Vec::new();
    let mut out2 = Vec::new();
    f.filter(&mut svg.as_bytes(), &mut out).unwrap();
    f.filter(&mut out.as_slice(), &mut out2).unwrap();
    assert_eq!(&out, &out2);
}
