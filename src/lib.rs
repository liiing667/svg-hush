//! Filters SVG files to make them safe to host anywhere.
//!
//! ```rust
//! use svg_hush::*;
//! let mut file = "<svg xmlns='http://www.w3.org/2000/svg' />".as_bytes();
//! let mut filter = Filter::new();
//! filter.set_data_url_filter(data_url_filter::allow_standard_images);
//! let mut out = Vec::new();
//! filter.filter(&mut file, &mut out)?;
//! # Ok::<_, FError>(())
//! ```

use crate::data_url_filter::DataUrl;
use crate::data_url_filter::DataUrlFilterResult;
use quick_error::quick_error;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::LazyLock;
use xml::attribute::{Attribute, OwnedAttribute};
use xml::EmitterConfig;
use xml::name::Name;
use xml::name::OwnedName;
use xml::ParserConfig;
use xml::reader::XmlEvent as REvent;
use xml::writer::XmlEvent as WEvent;


quick_error! {
    /// XML parsing error, I/O error, etc.
    #[derive(Debug)]
    pub enum FError {
        Reader(err: xml::reader::Error) {
            from()
            display("XML parsing error")
            source(err)
        }
        Writer(err: xml::writer::Error) {
            from()
            display("XML encoding error")
            source(err)
        }
        /// A nested `data:image/svg+xml` document was embedded deeper than [`Filter::set_max_nested_svg_depth`].
        NestedDepthLimitExceeded(depth: usize) {
            display("nested SVG depth limit ({depth}) exceeded")
        }
        /// A decoded `data:` URL body was larger than [`Filter::set_max_data_url_size`].
        DataUrlTooLarge(size: usize) {
            display("decoded data: URL is too large ({size} bytes)")
        }
        /// The total filtered output exceeded [`Filter::set_max_total_output_size`].
        OutputTooLarge(size: usize) {
            display("filtered SVG output is too large ({size} bytes)")
        }
    }
}

/// Default maximum nesting depth of `data:image/svg+xml` documents, see [`Filter::set_max_nested_svg_depth`].
pub const DEFAULT_MAX_NESTED_SVG_DEPTH: usize = 10;
/// Default maximum decoded size of a single `data:` URL body, see [`Filter::set_max_data_url_size`].
pub const DEFAULT_MAX_DATA_URL_SIZE: usize = 16 * 1024 * 1024;
/// Default maximum size of the total filtered output, see [`Filter::set_max_total_output_size`].
pub const DEFAULT_MAX_TOTAL_OUTPUT_SIZE: usize = 64 * 1024 * 1024;

/// Marker error used by the counting writer so an over-budget write can be mapped to [`FError::OutputTooLarge`].
#[derive(Debug)]
struct OutputBudgetExceeded {
    written: usize,
}

impl std::fmt::Display for OutputBudgetExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "filtered SVG output budget exceeded ({} bytes)", self.written)
    }
}

impl std::error::Error for OutputBudgetExceeded {}

/// An [`io::Write`] wrapper that counts every byte emitted and fails once the output budget is spent.
struct CountingWriter<W> {
    inner: W,
    written: usize,
    budget: usize,
}

impl<W: io::Write> CountingWriter<W> {
    fn new(inner: W, budget: usize) -> Self {
        Self { inner, written: 0, budget }
    }

    fn check_budget(&self) -> Result<(), io::Error> {
        if self.written > self.budget {
            Err(io::Error::other(OutputBudgetExceeded { written: self.written }))
        } else {
            Ok(())
        }
    }
}

impl<W: io::Write> io::Write for CountingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.written += n;
        self.check_budget()?;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

mod attrs;

/// Optionally, you can allow or modify `data:` URLs.
///
/// Please be careful with this, because these URLs can carry any file type.
pub mod data_url_filter;

enum Attr<'a> {
    /// Left unchanged
    Keep(Attribute<'a>),
    /// New attribute value
    Rewrite(OwnedAttribute),
    /// Attr is not allowed and must be discarded
    Drop,
}

enum ElementAction {
    Keep,
    FilterCSS,
    Drop,
}

#[derive(Copy, Clone)]
pub(crate) enum AttrType {
    /// Numbers, lengths, lists of numbers or keywords, and other non-URL non-script things
    AnyAscii,
    /// A single ASCII word
    Keyword,
    /// A floating-point number
    Number,
    /// Allows non-ASCII
    Text,
    /// Name of another attribute
    AttributeName,
    /// May be a URL (keywords are left alone, they're relative URLs at worst)
    Url,
    /// May contain "url(#)"
    UrlFunc,
    /// All kinds of dangers
    StyleSheet,
}

static ALLOWED_SVG_ELEMENTS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| [
    "a", // but href is checked separately
    "altGlyph",
    "altGlyphDef",
    "altGlyphItem",
    "animate",
    "animateColor",
    "animateMotion",
    "animateTransform",
    "circle",
    "clipPath",
    "color-profile",
    "defs",
    "desc",
    "ellipse",
    "feBlend",
    "feColorMatrix",
    "feComponentTransfer",
    "feComposite",
    "feConvolveMatrix",
    "feDiffuseLighting",
    "feDisplacementMap",
    "feDistantLight",
    "feDropShadow",
    "feFlood",
    "feFuncA",
    "feFuncB",
    "feFuncG",
    "feFuncR",
    "feGaussianBlur",
    "feImage",
    "feMerge",
    "feMergeNode",
    "feMorphology",
    "feOffset",
    "fePointLight",
    "feSpecularLighting",
    "feSpotLight",
    "feTile",
    "feTurbulence",
    "filter",
    "flowDiv",
    "flowLine",
    "flowPara",
    "flowRegion",
    "flowRoot",
    "flowSpan",
    "flowTref",
    "font",
    "g",
    "glyph",
    "glyphRef",
    "hkern",
    "image",
    "line",
    "linearGradient",
    "marker",
    "mask",
    "meshgradient",
    "meshpatch",
    "meshrow",
    "mpath",
    "mPath", // not sure about the case, dtd and tests disagre
    "path",
    "pattern",
    "polygon",
    "polyline",
    "radialGradient",
    "rect",
    "set",
    "solidColor",
    "stop",
    "svg",
    "switch",
    "symbol",
    "text",
    "textPath",
    "title",
    "tref",
    "tspan",
    "use",
    "view",
    "vkern",
].into());

static ATTRIBUTE_TYPES: LazyLock<HashMap<&'static str, AttrType>> = LazyLock::new(|| {
    let attribute_types: HashMap<_, _> = attrs::ATTRS.iter().copied().collect();
    assert_eq!(attribute_types.len(), attrs::ATTRS.len());
    attribute_types
});

/// The main entry point. Call [`Filter::new`]
pub struct Filter {
    /// May improve compression
    sort_attributes: bool,
    /// We're not allowing non-SVG namespaces anyway
    strip_prefixes: bool,
    image_filter: Option<Box<dyn for<'a> Fn(&'a DataUrl<'a>) -> DataUrlFilterResult>>,
    /// HTTP header
    content_type: Option<String>,
    /// How deep `data:image/svg+xml` documents may be nested before filtering fails
    max_nested_depth: usize,
    /// Maximum decoded size of a single `data:` URL body
    max_data_url_size: usize,
    /// Maximum size of the total filtered output, including rewritten nested SVG
    max_total_output_size: usize,
}

impl Filter {
    /// Create new filter instance. Call [`Filter::filter`] on it.
    #[must_use] 
    pub fn new() -> Self {
        Self {
            sort_attributes: true,
            strip_prefixes: true,
            image_filter: None,
            content_type: None,
            max_nested_depth: DEFAULT_MAX_NESTED_SVG_DEPTH,
            max_data_url_size: DEFAULT_MAX_DATA_URL_SIZE,
            max_total_output_size: DEFAULT_MAX_TOTAL_OUTPUT_SIZE,
        }
    }

    /// If parsing SVG from HTTP, provide the value of the HTTP header to read encoding from.
    pub fn set_content_type_header(&mut self, header: impl Into<String>) {
        self.content_type = Some(header.into());
    }

    /// Limit how many levels of nested `data:image/svg+xml` documents are sanitized recursively.
    ///
    /// The outer document is at depth 0. A nested SVG kept by the data URL filter is sanitized
    /// again with the same whitelist at depth 1, and so on. Exceeding this limit makes
    /// [`Filter::filter`] return [`FError::NestedDepthLimitExceeded`].
    ///
    /// Defaults to [`DEFAULT_MAX_NESTED_SVG_DEPTH`].
    pub fn set_max_nested_svg_depth(&mut self, depth: usize) {
        self.max_nested_depth = depth;
    }

    /// Limit the decoded size of a single `data:` URL body.
    ///
    /// Rewritten URLs and nested SVG documents are decoded before they can be rewritten, so this
    /// cap bounds memory used while processing them. Exceeding it makes [`Filter::filter`] return
    /// [`FError::DataUrlTooLarge`].
    ///
    /// Defaults to [`DEFAULT_MAX_DATA_URL_SIZE`].
    pub fn set_max_data_url_size(&mut self, size: usize) {
        self.max_data_url_size = size;
    }

    /// Limit the total number of bytes written to the output, including every rewritten nested
    /// `data:image/svg+xml` payload.
    ///
    /// Exceeding it makes [`Filter::filter`] return [`FError::OutputTooLarge`].
    /// Defaults to [`DEFAULT_MAX_TOTAL_OUTPUT_SIZE`].
    pub fn set_max_total_output_size(&mut self, size: usize) {
        self.max_total_output_size = size;
    }

    /// This function will be called for every `data:` URL encountered in embedded images.
    ///
    /// This callback is not allowed to refer to its environment. Use `Arc<Mutex<_>>` to modify any shared state.
    pub fn set_data_url_filter(&mut self, filter: impl Fn(&DataUrl) -> DataUrlFilterResult + 'static) {
        self.image_filter = Some(Box::new(filter));
    }

    fn is_allowed_element(&self, name: Name) -> ElementAction {
        if name.namespace == Some("http://www.w3.org/2000/svg") {
            if name.local_name == "style" {
                return ElementAction::FilterCSS;
            }
            if ALLOWED_SVG_ELEMENTS.get(&name.local_name).is_some() {
                return ElementAction::Keep;
            }
        }
        ElementAction::Drop
    }

    /// This list must not contain `<a>` and visible elements
    fn may_use_href(element_local_name: &str) -> bool {
        matches!(element_local_name, "radialGradient" | "linearGradient" | "image" | "use" | "pattern" | "feImage")
    }

    /// Read an SVG image from the `source` and write a filtered image to the `destination`.
    pub fn filter<Read: io::Read, Write: io::Write>(&self, source: Read, destination: Write) -> Result<(), FError> {
        let mut counting = CountingWriter::new(destination, self.max_total_output_size);
        let run = FilterRun { filter: self, depth: 0, use_content_type: true };
        let result = run.filter_document(source, &mut counting);
        if result.is_err() && counting.written > counting.budget {
            return Err(FError::OutputTooLarge(counting.written));
        }
        result
    }
}

/// State of a single (possibly nested) sanitization pass.
struct FilterRun<'a> {
    filter: &'a Filter,
    /// 0 for the outer document, incremented for every nested `data:image/svg+xml`.
    depth: usize,
    /// The `Content-Type` HTTP header only applies to the outer upload, never to nested data.
    use_content_type: bool,
}

impl<'a> FilterRun<'a> {
    fn filter_document<Read: io::Read, Write: io::Write>(
        &self,
        source: Read,
        destination: &mut CountingWriter<Write>,
    ) -> Result<(), FError> {
        let mut config = ParserConfig::new()
            .cdata_to_characters(true)
            .ignore_comments(true)
            .coalesce_characters(false)
            .allow_multiple_root_elements(false)
            .max_attributes(200)
            .max_entity_expansion_depth(3)
            .max_data_length(1<<28)
            .max_name_length(1000);
        if self.use_content_type {
            if let Some(ct) = &self.filter.content_type {
                config = config.content_type(ct);
            }
        }

        let parser = config.create_reader(source);

        let mut writer = EmitterConfig::new()
            .cdata_to_characters(true)
            .keep_element_names_stack(false)
            .autopad_comments(false)
            .perform_indent(true)
            .pad_self_closing(false)
            .create_writer(&mut *destination);

        let mut skipping = 0;
        let mut accumulating_css_text: Option<String> = None;
        let mut rewrite = Vec::new();
        let mut reached_document_end = false;
        let mut emitted_any_element = false;
        for e in parser {
            let mut e = e?;
            let w = match &mut e {
                REvent::StartElement {
                    name,
                    attributes,
                    namespace,
                } => {
                    if skipping > 0 {
                        skipping += 1;
                        continue;
                    }

                    match self.filter.is_allowed_element(name.borrow()) {
                        ElementAction::Keep => {},
                        ElementAction::FilterCSS if accumulating_css_text.is_none() => {
                            accumulating_css_text = Some(String::new());
                        },
                        _ => {
                            skipping += 1;
                            continue;
                        }
                    }

                    let mut keep = Vec::with_capacity(attributes.len());
                    rewrite.clear();

                    // xlink to href conversion could cause a duplicate if the element had both
                    let mut seen_href = false;
                    for a in attributes {
                        if a.name.local_name == "href" {
                            if seen_href {
                                continue;
                            }
                            seen_href = true;
                        }
                        match self.filter_attribute((*a).borrow(), &name.local_name)? {
                            Attr::Keep(a) => keep.push(a),
                            Attr::Rewrite(new_a) => rewrite.push(new_a),
                            Attr::Drop => {},
                        }
                    }
                    keep.extend(rewrite.iter().map(|a| a.borrow()));

                    if self.filter.sort_attributes {
                        keep.sort_by(|a, b| a.name.local_name.cmp(b.name.local_name));
                    }

                    // must be done in the end tag too
                    if self.filter.strip_prefixes {
                        namespace.0.retain(|k, _| k.is_empty());
                        name.prefix = None;
                    }

                    emitted_any_element = true;
                    WEvent::StartElement {
                        name: (*name).borrow(),
                        attributes: keep.into(),
                        namespace: Cow::Borrowed(&*namespace),
                    }
                }
                REvent::EndElement { name } => {
                    if skipping > 0 {
                        skipping -= 1;
                        continue;
                    }
                    if self.filter.strip_prefixes {
                        name.prefix = None;
                    }
                    if let Some(css) = accumulating_css_text.take() {
                        writer.write(WEvent::Characters(&self.filtered_url_func(&css)?))?;
                    }
                    WEvent::EndElement {
                        name: Some((*name).borrow()),
                    }
                }
                REvent::StartDocument {
                    version,
                    encoding: _,
                    standalone: _,
                } => {
                    debug_assert_eq!(0, skipping);
                    if skipping != 0 { break; }
                    WEvent::StartDocument {
                        version: *version,
                        encoding: Some("utf-8"),
                        standalone: None,
                    }
                }
                REvent::EndDocument => {
                    debug_assert_eq!(0, skipping);
                    if skipping != 0 { break; }
                    reached_document_end = true;
                    break;
                }

                REvent::ProcessingInstruction { .. } => continue,
                REvent::CData(_) | REvent::Comment(_) => unreachable!(),
                REvent::Characters(c) | REvent::Whitespace(c) => {
                    if skipping > 0 {
                        continue;
                    }
                    if let Some(css) = &mut accumulating_css_text {
                        css.push_str(c);
                        WEvent::Characters("")
                    } else {
                        WEvent::Characters(c)
                    }
                },
                REvent::Doctype { .. } => continue,
            };
            writer.write(w)?;
        }
        if reached_document_end && emitted_any_element {
            Ok(())
        } else {
            Err(xml::writer::Error::from(io::Error::new(io::ErrorKind::UnexpectedEof, "No acceptable SVG elements found")).into())
        }
    }

    fn filter_attribute<'b>(&self, attr: Attribute<'b>, element_local_name: &str) -> Result<Attr<'b>, FError> {
        // XML is weird in that attributes on a namespaced element are not in the element's namespace!
        // So all SVG attributes are in the default namespace.
        // We only allow the deprecated xlink:href, and only to downgrade it to a plain href.
        if let Some(ns) = attr.name.namespace {
            // TODO: support xml::space=preserve
            if ns != "http://www.w3.org/1999/xlink" || attr.name.local_name != "href" {
                return Ok(Attr::Drop);
            }
            debug_assert!(attr.name.prefix.is_some());
        }

        // Linking to same-origin images and gradients is fine (we'll filter the URL)
        if attr.name.local_name == "href" && !Filter::may_use_href(element_local_name) {
            return Ok(Attr::Drop);
        }

        use AttrType::*;
        let attr_type = match ATTRIBUTE_TYPES.get(&attr.name.local_name).copied() {
            Some(Text) => return Ok(Attr::Keep(attr)),
            Some(other) => other,
            // DOMPurify passes these through. ARIA is not supposed to have any scripting or remote resources.
            None if attr.name.local_name.starts_with("aria-") => AttrType::AnyAscii,
            None if attr.name.local_name.starts_with("data-") => return Ok(Attr::Keep(attr)),
            None => return Ok(Attr::Drop),
        };

        // From here these are all attributes containing code, so they're supposed to be ASCII and tolerate normalized whitespace.
        // Historically there have been cases of filter bypasses due to browsers silently skipping "invalid" characters,
        // or due to overeager Unicode support treating various fancy whitespace characters, CJK full-width letters, or non-ASCII punctuation as part of the syntax.
        // This strips values to bare ASCII to ensure that what we parse is a simple case most likely to match what the browsers see.
        // The value is already XML-decoded, so entities are not an issue here.
        let trimmed_value = attr.value.trim();
        let attr_value = if trimmed_value.bytes().all(|c| c.is_ascii() && c != 0) {
            Cow::Borrowed(trimmed_value)
        } else {
            Cow::Owned(
                attr.value
                    .chars()
                    .map(|c| if c.is_whitespace() { ' ' } else { c })
                    .filter(|&c| c.is_ascii() && c != '\0')
                    .collect(),
            )
        };

        /// makes a non-namespaced attribute
        fn no_ns_attr_with_value<'a>(attr: Attribute<'a>, value: Cow<str>) -> Attr<'a> {
            if attr.value == value && attr.name.namespace.is_none() && attr.name.prefix.is_none() {
                Attr::Keep(attr)
            } else {
                Attr::Rewrite(OwnedAttribute {
                    name: OwnedName { local_name: attr.name.local_name.to_owned(), namespace: None, prefix: None },
                    value: value.into_owned(),
                })
            }
        }

        match attr_type {
            // These checks aren't striclty necessary, just an integrity check
            AnyAscii | Keyword | Number | Text => Ok(no_ns_attr_with_value(attr, attr_value)),
            // this is from deprecated SMIL. Don't let it create a banned attribute by "animating" to it.
            AttributeName if matches!(ATTRIBUTE_TYPES.get(attr.value.trim()), Some(AnyAscii | Keyword | Number | Text))
                => Ok(no_ns_attr_with_value(attr, attr_value)),

            // Serious filtering starts here
            Url => match self.filter_url(&attr_value)? {
                Some(val) => Ok(no_ns_attr_with_value(attr, val.into())),
                None => Ok(Attr::Drop),
            },
            // The SVG spec says FuncIRI is a super simple `url(` token, but in browsers it's not. Browsers still support CSS-isms in url(),
            // so we'll just reuse CSS filter for them.
            UrlFunc => Ok(no_ns_attr_with_value(attr, self.filtered_url_func(&attr_value)?.into())),
            StyleSheet => Ok(no_ns_attr_with_value(attr, self.filtered_url_func(&attr_value)?.into())),
            _ => Ok(Attr::Drop),
        }
    }

    fn encode_data_url(mime_type: &str, data: &[u8]) -> String {
        use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine as _};
        let mut out = String::with_capacity(mime_type.len() + data.len() * 8 / 6 + 20);
        out.push_str("data:");
        out.push_str(mime_type);
        out.push_str(";base64,");
        STANDARD_NO_PAD.encode_string(data, &mut out);
        out
    }

    /// Run the same whitelist on a nested `data:image/svg+xml` document as on the outer SVG.
    fn sanitize_nested_svg(&self, data: &[u8]) -> Result<Vec<u8>, FError> {
        let depth = self.depth + 1;
        if depth > self.filter.max_nested_depth {
            return Err(FError::NestedDepthLimitExceeded(self.filter.max_nested_depth));
        }
        let nested = FilterRun { filter: self.filter, depth, use_content_type: false };
        let mut out = Vec::new();
        {
            let mut counting = CountingWriter::new(&mut out, self.filter.max_total_output_size);
            let result = nested.filter_document(data, &mut counting);
            if result.is_err() && counting.written > counting.budget {
                return Err(FError::OutputTooLarge(counting.written));
            }
            result?;
        }
        Ok(out)
    }

    fn filter_data_url(&self, url_str: &str) -> Result<Option<String>, FError> {
        let Some(f) = self.filter.image_filter.as_ref() else {
            return Ok(None);
        };

        let url = match DataUrl::process(url_str) {
            Ok(url) => url,
            // Malformed data URLs have always been silently dropped; keep that behavior.
            Err(_) => return Ok(None),
        };
        match f(&url) {
            DataUrlFilterResult::Drop => Ok(None),
            DataUrlFilterResult::Rewrite { mime_type, data } => {
                if data.len() > self.filter.max_data_url_size {
                    return Err(FError::DataUrlTooLarge(data.len()));
                }
                Ok(Some(Self::encode_data_url(&mime_type, &data)))
            },
            DataUrlFilterResult::Keep => {
                let is_svg = {
                    let mime = url.mime_type();
                    mime.type_ == "image" && mime.subtype == "svg+xml"
                };
                let (body, _fragment) = match url.decode_to_vec() {
                    Ok(decoded) => decoded,
                    // Un-decodable payloads used to be passed through as-is; preserve that.
                    Err(_) => return Ok(Some(url_str.to_owned())),
                };
                if body.len() > self.filter.max_data_url_size {
                    return Err(FError::DataUrlTooLarge(body.len()));
                }
                if is_svg {
                    let sanitized = self.sanitize_nested_svg(&body)?;
                    Ok(Some(Self::encode_data_url("image/svg+xml", &sanitized)))
                } else {
                    Ok(Some(url_str.to_owned()))
                }
            },
        }
    }

    /// Transforms URLs to relative (same-origin) URLs if possible. The host part is simply thrown away,
    /// since we're not concerned about causing 404s. Files that depend on cross-origin resources can still
    /// be made to work if the resources are copied to the same host as the file.
    fn filter_url(&self, url_str: &str) -> Result<Option<String>, FError> {
        // The URL crate won't parse relative URLs directly, and `make_relative()` requires a base too
        let base_url = url::Url::parse("https://127.0.0.1/__relpath_prefix__/").expect("base");
        let Some(url) = base_url.join(url_str).ok() else {
            return Ok(None);
        };
        if url.scheme() == "data" {
            return self.filter_data_url(url_str);
        }
        if url.cannot_be_a_base() {
            return Ok(None);
        }

        let path = url.path();
        let mut relative = path
            .strip_prefix("/__relpath_prefix__/")
            .unwrap_or(path)
            .to_string();
        if relative.trim_start().starts_with("//") {
            // path became scheme-relative URL
            return Ok(None);
        }
        // defence in depth to prevent url being interpreted as having a scheme
        if relative.contains(':') {
            relative = relative.replace(':', "%3a");
        }
        if let Some(query) = url.query() {
            relative.push('?');
            relative.push_str(query);
        }
        // this is super unlikely to be an image
        if relative == "/" {
            return Ok(None);
        }
        if let Some(fragment) = url.fragment() {
            relative.push('#');
            relative.push_str(fragment);
        }
        // in case our type data was wrong and Url type was UrlFunc type, break the url() syntax
        if relative.contains('(') {
            relative = relative.replace('(', "%28");
        }
        Ok(Some(relative))
    }

    /// ## Why is this parser so hacky?
    ///
    /// Filtering properly would require precisely tokenizing the input,
    /// which depends on supporting syntax of every SVG attribute and CSS.
    /// Any mismatch between actual syntax and the parser would risk these two getting out of sync,
    /// and enabling filter bypass (e.g. if our tokenizer saw a comment but browsers didn't).
    /// Proper implementation needs browser-grade parsers, and that's a big ask for a small tool.
    ///
    /// So here's a brute approach instead: replacing everything that could possibly be a `url()`,
    /// and throwing away everything that looks tricky to parse.
    fn filtered_url_func(&self, css_or_svg: &str) -> Result<String, FError> {
        let mut out = String::with_capacity(css_or_svg.len());

        // find next potential start of a URL (case-insensitive)
        let mut inside_url = false;
        for mut chunk in css_or_svg.split_inclusive('(') {
            if inside_url {
                // unescaped ( in URLs is not allowed, keep skipping until url( closes
                let Some((url, rest)) = chunk.split_once(')') else {
                    continue;
                };
                chunk = rest;

                let url = url.trim().trim_matches(['"', '\''])
                    // the quoted value allows whitespace too!
                    .trim();

                let url = self.filter_url(url)?
                    // no tricky chars please.
                    .filter(|url| !url.contains(['(', ')', '\'', '"', '\\', ' ']));
                // the url(#) must stay to keep the syntax valid
                out.push_str(url.as_deref().unwrap_or("#"));
                out.push(')');
                inside_url = false;
            }

            for chunk in chunk.split_inclusive([';', '{', '}', ',']) {
                if chunk.contains('\\') || (chunk.contains('@') && chunk.to_ascii_lowercase().contains("@import")) {
                    if let Some(b @ (b'{' | b'}')) = chunk.as_bytes().last().copied() {
                        out.push(b as char);
                    }
                    continue;
                }
                out.push_str(chunk);
                // `url()` doesn't allow whitespace before '('
                inside_url = chunk.len() >= 4 && chunk.get(chunk.len() - 4..).is_some_and(|c| c.eq_ignore_ascii_case("url("));
            }
        }
        if inside_url {
            debug_assert!(out.ends_with('('));
            out.push(')');
        }
        Ok(out)
    }
}

#[test]
fn urlfunc() {
    let r = FilterRun { filter: &Filter::new(), depth: 0, use_content_type: true };
    assert_eq!(r.filtered_url_func("hello, url(world)").unwrap(), "hello, url(world)");
    assert_eq!(r.filtered_url_func("hello, world").unwrap(), "hello, world");
    assert_eq!(r.filtered_url_func("hello, url(http://evil.com)").unwrap(), "hello, url(#)");
    assert_eq!(r.filtered_url_func("hello, url( http://evil.com )").unwrap(), "hello, url(#)");
    assert_eq!(r.filtered_url_func("hello, url( //evil.com )").unwrap(), "hello, url(#)");
    assert_eq!(r.filtered_url_func("url( /okay ), (), bye").unwrap(), "url(/okay), (), bye");
    assert_eq!(r.filtered_url_func("hello, url( unclosed").unwrap(), "hello, url()");
    assert_eq!(r.filtered_url_func("hello, url( ) url(    ) bork").unwrap(), "hello, url() url() bork");
    assert_eq!(r.filtered_url_func("hello, url('1' )url(  2  ) bork").unwrap(), "hello, url(1)url(2) bork");
    assert_eq!(r.filtered_url_func("hello, url('(' )").unwrap(), "hello, url()");
    assert_eq!(r.filtered_url_func("aurl(burl(").unwrap(), "aurl()");
    assert_eq!(r.filtered_url_func("uurl()rl(").unwrap(), "uurl()rl(");
}

#[test]
fn css() {
    let r = FilterRun { filter: &Filter::new(), depth: 0, use_content_type: true };
    assert_eq!(r.filtered_url_func("ok:url(data:text/plain;base64,AAA)").unwrap(), "ok:url(#)");
    assert_eq!(r.filtered_url_func("color: red; background: URL(X); huh").unwrap(), "color: red; background: URL(X); huh");
    assert_eq!(r.filtered_url_func("@import 'foo'; FONT-size: 1em;").unwrap(), " FONT-size: 1em;");
    assert_eq!(r.filtered_url_func("u;url(data:x);rl(hack)").unwrap(), "u;url(#);rl(hack)");
    assert_eq!(r.filtered_url_func("u;\\;rl(hack)").unwrap(), "u;rl(hack)");
    assert_eq!(r.filtered_url_func("u;\\,rl(hack)").unwrap(), "u;rl(hack)");
    assert_eq!(r.filtered_url_func("u;\\url(hack)").unwrap(), "u;hack)");
    assert_eq!(r.filtered_url_func("font-size: 1em; @Import 'foo';").unwrap(), "font-size: 1em;");
    assert_eq!(r.filtered_url_func("@\\69MporT 'foo';").unwrap(), "");
    assert_eq!(r.filtered_url_func("color: red; background: URL( url(); huh)").unwrap(), "color: red; background: URL(); huh)");
    assert_eq!(r.filtered_url_func("color: red; background: URL(//x/rel);").unwrap(), "color: red; background: URL(/rel);");
    assert_eq!(r.filtered_url_func("color: red; background: URL(data:xx);").unwrap(), "color: red; background: URL(#);");
    assert_eq!(r.filtered_url_func("color: red; background: UR\\L(x); border: blue").unwrap(), "color: red;x); border: blue");
    assert_eq!(r.filtered_url_func("prop: url (it is not);").unwrap(), "prop: url (it is not);");
    assert_eq!(r.filtered_url_func("sel, sel\\2 { bg: url(data:x); ba\\d; }").unwrap(), "sel,{ bg: url(#); }");
}

#[test]
fn data_url_filter() {
    let mut f = Filter::new();
    f.set_data_url_filter(|_data| {
        DataUrlFilterResult::Keep
    });
    let r = FilterRun { filter: &f, depth: 0, use_content_type: true };
    assert!(r.filter_url("data://wat").unwrap().is_none());
    assert_eq!(r.filter_url("data:text/plain,meh").unwrap().unwrap(), "data:text/plain,meh");
    drop(r);

    f.set_data_url_filter(|data| {
        assert_eq!(b"hello test"[..], data.decode_to_vec().unwrap().0);
        DataUrlFilterResult::Drop
    });
    let r = FilterRun { filter: &f, depth: 0, use_content_type: true };
    assert!(r.filter_url("data:text/plain,hello%20test").unwrap().is_none());
    drop(r);

    f.set_data_url_filter(|data| {
        assert_eq!(b"meh"[..], data.decode_to_vec().unwrap().0);
        DataUrlFilterResult::Rewrite { mime_type: "text/html".into(), data: "hi".into() }
    });
    let r = FilterRun { filter: &f, depth: 0, use_content_type: true };
    assert_eq!(r.filter_url("data:text/plain,meh").unwrap().unwrap(), "data:text/html;base64,aGk");
    drop(r);

    f.set_data_url_filter(|data| {
        assert_eq!(b"hi"[..], data.decode_to_vec().unwrap().0);
        DataUrlFilterResult::Keep
    });
    let r = FilterRun { filter: &f, depth: 0, use_content_type: true };
    assert_eq!(r.filter_url("data:text/html;base64,aGk#frag").unwrap().unwrap(), "data:text/html;base64,aGk#frag");
}

#[test]
fn namespace_uri_attribute_injection_blocked() {
    // Regression test for VULN-144180 / HackerOne #3814181:
    // Attribute injection via unescaped XML namespace URI. An input attribute
    // value containing &quot; is decoded by the XML parser to a literal " byte
    // in the namespace URI string; if the writer emits `xmlns="{uri}"` without
    // escaping, that " breaks out of the attribute and injects an onload
    // handler that svg-hush is supposed to strip.
    //
    // Upstream fix: the `xml` crate emitter (v1.4+) escapes namespace URIs
    // consistently with normal attribute values (Escaped::<AttributeEscapes>).
    // svg-hush depends on `xml 1.4` so the injected " becomes &quot; and stays
    // inside the xmlns="..." attribute value. The test catches a regression if
    // the dep is ever downgraded or the escaping is otherwise defeated.
    let malicious = br#"<s:svg xmlns:s="http://www.w3.org/2000/svg" xmlns="http://www.w3.org/2000/svg&quot; onload=&quot;alert(1)"><rect/></s:svg>"#;
    let mut out = Vec::new();
    Filter::new().filter(&malicious[..], &mut out).expect("filter succeeded");
    let s = std::str::from_utf8(&out).expect("utf-8");

    // The malicious content must remain inside the xmlns attribute value.
    // If the " byte in the URI ever emits unescaped, `onload="` (with a
    // literal quote) would appear as its own attribute. That must not happen.
    assert!(
        !s.contains(r#"onload=""#),
        "svg-hush emitted an injected onload attribute (unescaped): {s}"
    );
}

#[test]
fn url_filter() {
    let r = FilterRun { filter: &Filter::new(), depth: 0, use_content_type: true };
    assert_eq!(r.filter_url("http://test.com/a.jpg").unwrap().unwrap(), "/a.jpg");
    assert_eq!(
        r.filter_url("https://test.com:123/.././a/b/c.jpg").unwrap().unwrap(),
        "/a/b/c.jpg"
    );
    assert_eq!(
        r.filter_url("/hello world.jpg").unwrap().unwrap(),
        "/hello%20world.jpg"
    );
    assert_eq!(r.filter_url("b.jpg").unwrap().unwrap(), "b.jpg");
    assert_eq!(r.filter_url("./x/").unwrap().unwrap(), "x/");
    assert_eq!(r.filter_url("#hash").unwrap().unwrap(), "#hash");
    assert_eq!(r.filter_url("?q s").unwrap().unwrap(), "?q%20s");
    assert_eq!(r.filter_url("//host/PAth").unwrap().unwrap(), "/PAth");
    assert_eq!(r.filter_url("//host/%2fpath").unwrap().unwrap(), "/%2fpath");
    assert_eq!(r.filter_url("//host%%%/path").unwrap(), None);
    assert_eq!(r.filter_url("//host///path").unwrap(), None);
    assert_eq!(r.filter_url("data:text/html,xx").unwrap(), None);
    assert_eq!(r.filter_url("blob:123").unwrap(), None);
    assert_eq!(r.filter_url("javascript:alert(1)").unwrap(), None);
    assert_eq!(r.filter_url("jAvascript: alert(1)").unwrap(), None);
    assert_eq!(r.filter_url("  jAvascript: alert(1) //http://").unwrap(), None);
}
