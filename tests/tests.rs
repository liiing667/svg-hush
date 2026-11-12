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

#[test]
fn css_escape_bypass() {
    // Browsers decode CSS escapes before matching url()/@import, so escaped
    // identifiers must not hide external resources from the filter.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg">
    <style>.a { fill: u\72 l(http://evil.example) } @\69 mport "//evil.example/x.css";</style>
    <rect style="fill: \75\72\6c(http://evil.example); background: URL\28 //evil.example/x)"/>
    <set attributeName="fill" to="u\72 l(http://evil.example)"/>
    <rect style="fill: \5c 75\5c 72\5c 6c(http://evil.example)"/>
</svg>"##;
    let f = Filter::new();
    let mut out = Vec::new();
    f.filter(&mut svg.as_bytes(), &mut out).unwrap();
    let once = String::from_utf8(out).unwrap();
    assert!(!once.contains("evil.example"), "escaped external resource survived: {once}");
    assert!(once.contains("url(#)"), "{once}");

    // the output must be stable when filtered again
    let mut out2 = Vec::new();
    f.filter(&mut once.as_bytes(), &mut out2).unwrap();
    assert_eq!(once, String::from_utf8(out2).unwrap());
}

#[test]
fn css_escape_legit() {
    // legitimate escapes keep their meaning (decoded to equivalent text)
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg">
    <style>.\31 23 { fill: url(/ok\61 y) }</style>
    <rect style="content: '\2014'; fill: red"/>
</svg>"##;
    let f = Filter::new();
    let mut out = Vec::new();
    f.filter(&mut svg.as_bytes(), &mut out).unwrap();
    let once = String::from_utf8(out).unwrap();
    assert!(once.contains(".123 { fill: url(/okay) }"), "{once}");
    assert!(once.contains(r"content: &apos;\2014 &apos;; fill: red"), "{once}");

    let mut out2 = Vec::new();
    f.filter(&mut once.as_bytes(), &mut out2).unwrap();
    assert_eq!(once, String::from_utf8(out2).unwrap());
}
