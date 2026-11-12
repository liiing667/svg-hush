use svg_hush::*;
use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine as _};

fn filter_with_standard_images(svg: &[u8]) -> Result<Vec<u8>, FError> {
    let mut f = Filter::new();
    f.set_data_url_filter(data_url_filter::allow_standard_images);
    let mut out = Vec::new();
    f.filter(svg, &mut out)?;
    Ok(out)
}

fn svg_data_url(svg: &[u8]) -> String {
    format!("data:image/svg+xml;base64,{}", STANDARD_NO_PAD.encode(svg))
}

fn outer_svg(data_url: &str) -> Vec<u8> {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg"><image href="{data_url}"/></svg>"#
    )
    .into_bytes()
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

#[test]
fn nested_svg_is_sanitized_with_same_whitelist() {
    let nested = br#"<svg xmlns="http://www.w3.org/2000/svg" onload="alert(1)"><script>alert(2)</script><rect width="10" height="10"/><a href="http://evil.example/x">link</a></svg>"#;
    let outer = outer_svg(&svg_data_url(nested));

    let out = filter_with_standard_images(&outer).unwrap();
    let out = std::str::from_utf8(&out).unwrap();

    // Scripting and the event handler must be gone from the nested payload.
    assert!(!out.contains("onload"));
    assert!(!out.contains("<script"));
    assert!(!out.contains("alert"));
    // The whitelisted shape survives (base64-encoded in the output), so the icon is not lost.
    let start = out.find("data:image/svg+xml;base64,").unwrap();
    let end = out[start..].find('"').unwrap();
    let payload = &out[start + "data:image/svg+xml;base64,".len()..start + end];
    let decoded = STANDARD_NO_PAD.decode(payload).unwrap();
    let sanitized_nested = std::str::from_utf8(&decoded).unwrap();
    assert!(sanitized_nested.contains("<rect"));
    assert!(!sanitized_nested.contains("onload"));
    assert!(!sanitized_nested.contains("<script"));
    assert!(!sanitized_nested.contains("alert"));

    // Run the same filter on the rewritten data: URL again: the result must be stable.
    assert!(!sanitized_nested.contains("http://evil.example"));

    let mut again = Vec::new();
    let mut f = Filter::new();
    f.set_data_url_filter(data_url_filter::allow_standard_images);
    f.filter(&decoded[..], &mut again).unwrap();
    assert_eq!(again, decoded);
}

#[test]
fn nested_png_jpeg_gif_are_kept() {
    for mime in ["image/png", "image/jpeg", "image/gif"] {
        let data_url = format!("data:{mime};base64,{}", STANDARD_NO_PAD.encode(b"bin"));
        let out = filter_with_standard_images(&outer_svg(&data_url)).unwrap();
        let out = std::str::from_utf8(&out).unwrap();
        assert!(out.contains(&data_url), "kept {mime} data URL");
    }
}

#[test]
fn non_image_nested_data_urls_are_dropped() {
    let data_url = "data:text/html;base64,PGh0bWw+";
    let out = filter_with_standard_images(&outer_svg(data_url)).unwrap();
    let out = std::str::from_utf8(&out).unwrap();
    assert!(!out.contains("text/html"));
}

#[test]
fn non_data_url_behavior_is_unchanged() {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg">
        <image href="https://tracker.example.test/pixel.png"/>
        <use href="#shape"/>
        <rect id="shape" width="3" height="4"/>
    </svg>"##;
    let out = filter_with_standard_images(svg).unwrap();
    let out = std::str::from_utf8(&out).unwrap();
    assert!(!out.contains("tracker.example.test"));
    assert!(out.contains("/pixel.png"));
    assert!(out.contains("href=\"#shape\""));
}

#[test]
fn nested_svg_depth_limit_fails_explicitly() {
    let depth_limit = 2;
    let mut innermost = br#"<svg xmlns="http://www.w3.org/2000/svg"><circle xmlns="http://www.w3.org/2000/svg" r="1"/></svg>"#.to_vec();
    // Nesting level counts the outer document as 0, so create limit + 1 nested levels.
    for _ in 0..=depth_limit {
        innermost = outer_svg(&svg_data_url(&innermost));
    }

    let mut f = Filter::new();
    f.set_data_url_filter(data_url_filter::allow_standard_images);
    f.set_max_nested_svg_depth(depth_limit);
    let mut out = Vec::new();
    let err = f.filter(&innermost[..], &mut out).unwrap_err();
    assert!(matches!(err, FError::NestedDepthLimitExceeded(depth) if depth == depth_limit));

    // The same payload is accepted when the depth is raised.
    f.set_max_nested_svg_depth(depth_limit + 1);
    let mut out = Vec::new();
    f.filter(&innermost[..], &mut out).unwrap();
    let out = std::str::from_utf8(&out).unwrap();
    let start = out.find("data:image/svg+xml;base64,").unwrap();
    let rest = &out[start + "data:image/svg+xml;base64,".len()..];
    let payload = rest.split('"').next().unwrap();
    let decoded = STANDARD_NO_PAD.decode(payload).unwrap();
    // The first nested level survived (the icon chain is kept); the circle is one level deeper.
    let first_level = std::str::from_utf8(&decoded).unwrap();
    assert!(first_level.contains("data:image/svg+xml;base64,"));
    assert!(first_level.contains("<image"));
}

#[test]
fn decoded_data_url_size_limit_fails_explicitly() {
    let nested = br#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="64" height="64"/></svg>"#.to_vec();
    let nested_len = nested.len();
    let outer = outer_svg(&svg_data_url(&nested));

    let mut f = Filter::new();
    f.set_data_url_filter(data_url_filter::allow_standard_images);
    f.set_max_data_url_size(nested_len - 1);
    let mut out = Vec::new();
    let err = f.filter(&outer[..], &mut out).unwrap_err();
    assert!(matches!(err, FError::DataUrlTooLarge(size) if size == nested_len));

    f.set_max_data_url_size(nested_len);
    let mut out = Vec::new();
    f.filter(&outer[..], &mut out).unwrap();
}

#[test]
fn total_output_budget_fails_explicitly() {
    let nested = br#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="100" height="100"/></svg>"#;
    let outer = outer_svg(&svg_data_url(nested));

    let mut f = Filter::new();
    f.set_data_url_filter(data_url_filter::allow_standard_images);
    f.set_max_total_output_size(8);
    let mut out = Vec::new();
    let err = f.filter(&outer[..], &mut out).unwrap_err();
    assert!(matches!(err, FError::OutputTooLarge(_)));
}
