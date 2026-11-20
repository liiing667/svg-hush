use svg_hush::{FError, Filter, Limits};

/// Builds an SVG whose root contains `child_count` allowed `<rect/>` elements.
/// The element count of the resulting document is `child_count + 1` (plus root).
fn wide_svg(child_count: usize, attrs_per_child: usize) -> Vec<u8> {
    let mut doc = String::from(r#"<svg xmlns="http://www.w3.org/2000/svg">"#);
    for i in 0..child_count {
        doc.push_str("<rect");
        for j in 0..attrs_per_child {
            doc.push_str(&format!(" data-a{j}=\"{i}-{j}\""));
        }
        doc.push_str("/>");
    }
    doc.push_str("</svg>");
    doc.into_bytes()
}

fn filter_with_limits(doc: &[u8], limits: Limits) -> (Result<(), FError>, Vec<u8>) {
    let mut filter = Filter::new();
    filter.set_limits(limits);
    let mut out = Vec::new();
    let result = filter.filter(&mut &doc[..], &mut out);
    (result, out)
}

#[test]
fn document_inside_budgets_succeeds() {
    const CHILDREN: usize = 10;
    const ATTRS_PER_CHILD: usize = 2;
    let doc = wide_svg(CHILDREN, ATTRS_PER_CHILD);

    // Boundary: exactly the number of elements and attributes the parser reports.
    let limits = Limits {
        max_elements: CHILDREN + 1,
        max_attributes: CHILDREN * ATTRS_PER_CHILD,
        max_output_bytes: usize::MAX,
    };
    let (result, out) = filter_with_limits(&doc, limits);
    assert!(
        result.is_ok(),
        "document at the exact budget boundary must succeed: {result:?}"
    );

    // Compatibility: the filtered output is still valid and stays idempotent.
    let mut again = Vec::new();
    Filter::new()
        .filter(&mut out.as_slice(), &mut again)
        .unwrap();
    assert_eq!(
        out, again,
        "filtered output must remain a valid, stable SVG"
    );
    assert!(std::str::from_utf8(&out).unwrap().matches("<rect").count() == CHILDREN);
}

#[test]
fn element_budget_fails_closed() {
    const CHILDREN: usize = 10;
    let doc = wide_svg(CHILDREN, 0);

    let limits = Limits {
        max_elements: CHILDREN, // one short: root + CHILDREN rects
        max_attributes: usize::MAX,
        max_output_bytes: usize::MAX,
    };
    let (result, out) = filter_with_limits(&doc, limits);
    assert!(
        matches!(result, Err(FError::ElementLimitExceeded)),
        "got {result:?}"
    );
    // Fail closed: the over-limit document must not be left behind as a success.
    assert!(
        !out.windows(6).any(|w| w == b"</svg>"),
        "complete document must never be emitted on limit failure"
    );
}

#[test]
fn attribute_budget_fails_closed() {
    const CHILDREN: usize = 10;
    const ATTRS_PER_CHILD: usize = 3;
    let doc = wide_svg(CHILDREN, ATTRS_PER_CHILD);

    let limits = Limits {
        max_elements: usize::MAX,
        max_attributes: CHILDREN * ATTRS_PER_CHILD - 1,
        max_output_bytes: usize::MAX,
    };
    let (result, _out) = filter_with_limits(&doc, limits);
    assert!(
        matches!(result, Err(FError::AttributeLimitExceeded)),
        "got {result:?}"
    );
}

#[test]
fn output_budget_fails_closed_and_caps_bytes() {
    let doc = wide_svg(50, 2);
    let cap = 128usize;

    let limits = Limits {
        max_elements: usize::MAX,
        max_attributes: usize::MAX,
        max_output_bytes: cap,
    };
    let (result, out) = filter_with_limits(&doc, limits);
    assert!(
        matches!(result, Err(FError::OutputLimitExceeded)),
        "got {result:?}"
    );
    // The destination never receives more than the configured byte budget.
    assert!(
        out.len() <= cap,
        "wrote {} bytes despite a {cap} byte budget",
        out.len()
    );
    assert!(
        !out.windows(6).any(|w| w == b"</svg>"),
        "truncated output must not look like a finished document"
    );
}

#[test]
fn dropped_subtree_elements_still_count() {
    // A wide tree hidden inside a disallowed element must still be counted,
    // otherwise the DoS would happen before any output is produced.
    let mut doc = String::from(r#"<svg xmlns="http://www.w3.org/2000/svg"><script>"#);
    for _ in 0..20 {
        doc.push_str("<rect/>");
    }
    doc.push_str("</script></svg>");

    let limits = Limits {
        max_elements: 5,
        max_attributes: usize::MAX,
        max_output_bytes: usize::MAX,
    };
    let (result, _out) = filter_with_limits(doc.as_bytes(), limits);
    assert!(
        matches!(result, Err(FError::ElementLimitExceeded)),
        "got {result:?}"
    );
}

#[test]
fn distinct_error_variants_identify_failing_stage() {
    let doc = wide_svg(5, 1);

    let element = filter_with_limits(
        &doc,
        Limits {
            max_elements: 1,
            max_attributes: usize::MAX,
            max_output_bytes: usize::MAX,
        },
    )
    .0;
    let attribute = filter_with_limits(
        &doc,
        Limits {
            max_elements: usize::MAX,
            max_attributes: 0,
            max_output_bytes: usize::MAX,
        },
    )
    .0;
    let output = filter_with_limits(
        &doc,
        Limits {
            max_elements: usize::MAX,
            max_attributes: usize::MAX,
            max_output_bytes: 1,
        },
    )
    .0;

    assert!(matches!(element, Err(FError::ElementLimitExceeded)));
    assert!(matches!(attribute, Err(FError::AttributeLimitExceeded)));
    assert!(matches!(output, Err(FError::OutputLimitExceeded)));
    assert_ne!(
        format!("{:?}", element.unwrap_err()),
        format!("{:?}", output.unwrap_err())
    );
}

#[test]
fn default_limits_accept_normal_sized_documents() {
    let doc = wide_svg(100, 2);
    let (result, _out) = filter_with_limits(&doc, Limits::default());
    assert!(
        result.is_ok(),
        "ordinary SVG must stay compatible with default budgets: {result:?}"
    );
}

#[test]
fn output_budget_also_covers_css_flush() {
    // CSS text is buffered until the `style` end tag and flushed in one write.
    // The output wrapper must turn that budget breach into the same fail-closed error.
    let css = "a { fill: #000; }".repeat(100);
    let doc =
        format!(r#"<svg xmlns="http://www.w3.org/2000/svg"><style>{css}</style><rect/></svg>"#);

    let limits = Limits {
        max_elements: usize::MAX,
        max_attributes: usize::MAX,
        max_output_bytes: 64,
    };
    let (result, out) = filter_with_limits(doc.as_bytes(), limits);
    assert!(
        matches!(result, Err(FError::OutputLimitExceeded)),
        "got {result:?}"
    );
    assert!(out.len() <= 64);
}
