use svg_hush::*;

fn filter_to_string(svg: &str) -> String {
    let mut out = Vec::new();
    Filter::new()
        .filter(svg.as_bytes(), &mut out)
        .expect("filter succeeds");
    String::from_utf8(out).unwrap()
}

#[test]
fn comment_split_external_resources_are_neutralized() {
    // A block comment can split `@import` or the `url(` token across parser chunks.
    // Browsers remove comments before tokenizing, so the reassembled tokens must be
    // filtered identically in <style>, style="" attributes, and SMIL animation values.
    let malicious = [
        "<svg xmlns='http://www.w3.org/2000/svg'><style>@im/*x*/port 'http://evil.com/a.css';</style></svg>",
        "<svg xmlns='http://www.w3.org/2000/svg'><style>x{background:ur/*c*/l(http://evil.com/x.png)}</style></svg>",
        "<svg xmlns='http://www.w3.org/2000/svg'><rect style=\"background:ur/*c*/l(http://evil.com/x.png)\"/></svg>",
        "<svg xmlns='http://www.w3.org/2000/svg'><rect><animate attributeName='fill' to='ur/*c*/l(http://evil.com/x)'/></rect></svg>",
        "<svg xmlns='http://www.w3.org/2000/svg'><style>@import 'http://evil.com/a.css'; /* unclosed</style></svg>",
    ];
    for svg in malicious {
        let out = filter_to_string(svg).to_ascii_lowercase();
        assert!(
            !out.contains("evil.com"),
            "external reference survived in {svg}: {out}"
        );
        assert!(
            !out.contains("@import"),
            "@import survived in {svg}: {out}"
        );
    }
}

#[test]
fn benign_commented_css_is_preserved() {
    // Normal, same-origin CSS containing comments must keep working: comments are removed
    // (as browsers do) but the declarations and relative URLs survive.
    let benign = [
        "<svg xmlns='http://www.w3.org/2000/svg'><style>/* palette */ x{color:red}</style></svg>",
        "<svg xmlns='http://www.w3.org/2000/svg'><style>x{background:ur/* keep */l(img.png)}</style></svg>",
        "<svg xmlns='http://www.w3.org/2000/svg'><rect style='color:red; /* note */ fill:url(#g)'/></svg>",
        "<svg xmlns='http://www.w3.org/2000/svg'><rect><animate attributeName='fill' to='ur/* */l(#g)'/></rect></svg>",
    ];
    for svg in benign {
        let out = filter_to_string(svg);
        assert!(
            !out.contains("/*"),
            "comment should have been normalized away in {svg}: {out}"
        );
        assert!(
            out.to_ascii_lowercase().contains("color:red") || out.contains("img.png") || out.contains("url(#g)"),
            "benign CSS was over-filtered in {svg}: {out}"
        );
    }
}

#[test]
fn malicious_and_benign_outputs_differ() {
    // Same document shape, only the URL origin/comment trick differs: the malicious variant
    // loses its external reference while the benign relative reference is retained.
    let malicious = "<svg xmlns='http://www.w3.org/2000/svg'><style>x{background:ur/*c*/l(http://evil.com/x.png)}</style></svg>";
    let benign = "<svg xmlns='http://www.w3.org/2000/svg'><style>x{background:ur/*c*/l(x.png)}</style></svg>";

    let bad = filter_to_string(malicious);
    let good = filter_to_string(benign);

    assert!(!bad.to_ascii_lowercase().contains("evil.com"));
    assert!(good.contains("url(x.png)"));
    assert_ne!(bad, good);
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
