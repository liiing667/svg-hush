use svg_hush::*;

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

fn filter_str(svg: &str) -> Result<String, FError> {
    let mut out = Vec::new();
    Filter::new().filter(&mut svg.as_bytes(), &mut out)?;
    Ok(String::from_utf8(out).unwrap())
}

#[test]
fn local_refs_forward_and_backward() {
    // References may point to ids defined earlier or later in the document;
    // the closure is validated against the complete filtered document.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg">
        <rect fill="url(#g1)" width="10" height="10"/>
        <use href="#shape"/>
        <defs>
            <linearGradient id="g1"><stop offset="0" stop-color="#fff"/></linearGradient>
            <path id="shape" d="M0 0h10v10z"/>
        </defs>
        <rect fill="url(#g1)" width="5" height="5"/>
        <use href="#shape"/>
    </svg>"##;
    let out = filter_str(svg).expect("valid forward and backward references must pass");
    assert!(out.contains(r##"fill="url(#g1)""##), "{out}");
    assert!(out.contains(r##"href="#shape""##), "{out}");
    assert!(out.contains(r##"id="g1""##), "{out}");
    assert!(out.contains(r##"id="shape""##), "{out}");
}

#[test]
fn dangling_local_ref_rejected() {
    // url(#…) in a presentation attribute
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg"><rect fill="url(#nope)" width="10" height="10"/></svg>"##;
    assert!(matches!(filter_str(svg), Err(FError::DanglingReference(f)) if f == "nope"));

    // href="#…"
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg"><use href="#gone"/></svg>"##;
    assert!(matches!(filter_str(svg), Err(FError::DanglingReference(f)) if f == "gone"));

    // url(#…) in a <style> element
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg"><style>rect { fill: url(#missing); }</style><rect width="1" height="1"/></svg>"##;
    assert!(matches!(filter_str(svg), Err(FError::DanglingReference(f)) if f == "missing"));

    // url(#…) in a style="" attribute
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg"><rect style="fill:url(#missing)" width="1" height="1"/></svg>"##;
    assert!(matches!(filter_str(svg), Err(FError::DanglingReference(f)) if f == "missing"));

    // A reference to an id that only exists in a dropped subtree is dangling in the output
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg"><foreignObject><g id="hidden"/></foreignObject><use href="#hidden"/></svg>"##;
    assert!(matches!(filter_str(svg), Err(FError::DanglingReference(f)) if f == "hidden"));

    // Rejection must fail closed: nothing may be written to the destination
    let mut out = Vec::new();
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg"><rect fill="url(#nope)"/></svg>"##;
    assert!(Filter::new().filter(&mut svg.as_bytes(), &mut out).is_err());
    assert!(out.is_empty());
}

#[test]
fn duplicate_id_rejected() {
    // Ambiguous: two elements with the referenced id
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg">
        <defs><linearGradient id="g"/><linearGradient id="g"/></defs>
        <rect fill="url(#g)" width="10" height="10"/>
    </svg>"##;
    assert!(matches!(filter_str(svg), Err(FError::DuplicateId(id)) if id == "g"));

    // Duplicate ids are ambiguous even when nothing references them
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg"><g id="x"/><g id="x"/></svg>"##;
    assert!(matches!(filter_str(svg), Err(FError::DuplicateId(id)) if id == "x"));
}

#[test]
fn dropped_content_refs_ignored() {
    // References inside removed subtrees don't exist in the output and must not fail validation.
    // The url(#) placeholder used for filtered-out URLs is an empty fragment, not a local ref.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg">
        <script><rect fill="url(#whatever)"/></script>
        <rect fill="url(http://example.invalid/x.svg#p)" width="10" height="10"/>
        <rect fill="url(data:text/plain,x)" width="10" height="10"/>
        <image href="https://example.invalid/pic.png#frag"/>
    </svg>"##;
    let out = filter_str(svg).expect("references in dropped content must be ignored");
    assert!(out.contains("url(/x.svg#p)"), "{out}");
    assert!(out.contains("url(#)"), "{out}");
    assert!(out.contains(r##"href="/pic.png#frag""##), "{out}");
}
