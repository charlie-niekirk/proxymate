//! Deep filter terms: header and body predicates of the filter DSL, which
//! need a flow's detail or decoded bodies rather than its summary. A view
//! with any runs them as a background job ([`crate::Engine`]'s deep search)
//! that streams matching flows into the view; MCP evaluates them inline.

use pm_model::{BodyPart, FlowDetail, FlowId, Header};
use regex::bytes::{Regex, RegexBuilder};

use super::BlobBackend;
use crate::engine::body_head;

/// Bodies are searched up to this many decoded bytes.
pub(crate) const DEEP_BODY_MAX: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Request,
    Response,
    Both,
}

/// One `header:` or `body:` term: a case-insensitive substring.
#[derive(Debug, Clone)]
pub struct DeepTerm {
    what: What,
    side: Side,
    needle: Regex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum What {
    Header,
    Body,
}

impl DeepTerm {
    pub(crate) fn header(side: Side, needle: &str) -> DeepTerm {
        DeepTerm::new(What::Header, side, needle)
    }

    pub(crate) fn body(side: Side, needle: &str) -> DeepTerm {
        DeepTerm::new(What::Body, side, needle)
    }

    fn new(what: What, side: Side, needle: &str) -> DeepTerm {
        let needle = RegexBuilder::new(&regex::escape(needle))
            .case_insensitive(true)
            .unicode(false)
            .build()
            .expect("an escaped literal compiles");
        DeepTerm { what, side, needle }
    }

    fn parts(&self) -> &'static [BodyPart] {
        match self.side {
            Side::Request => &[BodyPart::Request],
            Side::Response => &[BodyPart::Response],
            Side::Both => &[BodyPart::Request, BodyPart::Response],
        }
    }
}

/// The deep terms of one query, each possibly negated; all must hold.
#[derive(Debug, Clone)]
pub struct DeepQuery {
    terms: Vec<(bool, DeepTerm)>,
}

impl DeepQuery {
    pub(crate) fn new(terms: Vec<(bool, DeepTerm)>) -> DeepQuery {
        DeepQuery { terms }
    }

    pub fn len(&self) -> usize {
        self.terms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// True if the flow's detail and bodies satisfy every term. A flow
    /// whose detail is gone (evicted, or synthetic) matches nothing.
    pub(crate) fn matches(&self, backend: &dyn BlobBackend, id: FlowId) -> bool {
        let Ok(Some(detail)) = backend.detail(id) else {
            return false;
        };
        let mut bodies: [Option<Vec<u8>>; 2] = [None, None];
        self.terms.iter().all(|(negate, term)| {
            let found = term.parts().iter().any(|&part| match term.what {
                What::Header => headers(&detail, part).iter().any(|h| header_matches(&term.needle, h)),
                What::Body => {
                    let body = bodies[part as usize].get_or_insert_with(|| load_body(backend, &detail, part));
                    term.needle.is_match(body)
                }
            });
            found != *negate
        })
    }
}

fn headers(detail: &FlowDetail, part: BodyPart) -> &[Header] {
    match part {
        BodyPart::Request => &detail.request.headers,
        BodyPart::Response => detail.response.as_ref().map_or(&[], |r| &r.headers),
    }
}

fn header_matches(needle: &Regex, h: &Header) -> bool {
    // Most needles fall inside the name or the value; only one spanning the
    // separator needs the joined line.
    needle.is_match(h.name.as_bytes())
        || needle.is_match(h.value.as_bytes())
        || needle.is_match(format!("{}: {}", h.name, h.value).as_bytes())
}

/// The decoded body (its start), or the raw bytes if it doesn't decode.
fn load_body(backend: &dyn BlobBackend, detail: &FlowDetail, part: BodyPart) -> Vec<u8> {
    let info = match part {
        BodyPart::Request => &detail.request_body,
        BodyPart::Response => &detail.response_body,
    };
    if info.captured == 0 {
        return Vec::new();
    }
    match body_head(backend, detail.id, part, true, Some(DEEP_BODY_MAX)) {
        Ok(body) => body.bytes,
        Err(_) => body_head(backend, detail.id, part, false, Some(DEEP_BODY_MAX))
            .map(|b| b.bytes)
            .unwrap_or_default(),
    }
}
