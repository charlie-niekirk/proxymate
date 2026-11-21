//! A `multipart/*` body split into its parts (`lib/body/multipart.ts`).

use std::ops::Range;

#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub headers: Vec<(String, String)>,
    /// `name` from Content-Disposition.
    pub name: Option<String>,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    /// Where the part's body is in the whole body.
    pub body: Range<usize>,
}

/// The `boundary` parameter of a multipart Content-Type.
pub fn boundary_of(content_type: Option<&str>) -> Option<String> {
    let ct = content_type?;
    let lower = ct.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find("boundary=") {
        let start = from + at;
        let before = lower[..start].trim_end();
        from = start + 9;
        if !before.ends_with(';') {
            continue;
        }
        let value = &ct[from..];
        return if let Some(quoted) = value.strip_prefix('"') {
            quoted.split('"').next().filter(|s| !s.is_empty()).map(str::to_owned)
        } else {
            value
                .split(|c: char| c == ';' || c.is_whitespace())
                .next()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
    }
    None
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from > hay.len() {
        return None;
    }
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// A Content-Disposition parameter (`name`, `filename`), unquoted.
fn param(value: &str, name: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find(name) {
        let start = from + at;
        from = start + name.len();
        if !lower[..start].trim_end().ends_with(';') {
            continue;
        }
        let mut rest = &value[from..];
        rest = rest.strip_prefix('*').unwrap_or(rest);
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        if let Some(quoted) = rest.strip_prefix('"') {
            let mut out = String::new();
            let mut chars = quoted.chars();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => out.extend(chars.next()),
                    '"' => return Some(out),
                    c => out.push(c),
                }
            }
            return Some(out);
        }
        return Some(rest.split(';').next().unwrap_or("").trim().to_owned());
    }
    None
}

/// Splits `body` on `boundary`. Tolerates LF-only line ends and a missing
/// closing delimiter (a cut capture keeps its last, partial part).
pub fn parse_multipart(body: &[u8], boundary: &str) -> Vec<Part> {
    let delimiter = format!("--{boundary}").into_bytes();
    let mut parts = Vec::new();
    let mut pos = find(body, &delimiter, 0);
    while let Some(p) = pos {
        let mut start = p + delimiter.len();
        if body.get(start..start + 2) == Some(b"--") {
            break; // the closing delimiter
        }
        while matches!(body.get(start), Some(b' ' | b'\t')) {
            start += 1;
        }
        if body.get(start) == Some(&b'\r') {
            start += 1;
        }
        if body.get(start) == Some(&b'\n') {
            start += 1;
        }
        let next = find(body, &delimiter, start);
        let mut end = next.unwrap_or(body.len());
        if next.is_some() {
            if end > 0 && body[end - 1] == b'\n' {
                end -= 1;
            }
            if end > 0 && body[end - 1] == b'\r' {
                end -= 1;
            }
        }
        let end = end.max(start);
        let chunk = &body[start..end];
        let (head_end, sep) = if chunk.starts_with(b"\r\n") {
            (Some(0), 2)
        } else if chunk.starts_with(b"\n") {
            (Some(0), 1)
        } else if let Some(at) = find(chunk, b"\r\n\r\n", 0) {
            (Some(at), 4)
        } else {
            (find(chunk, b"\n\n", 0), 2)
        };
        let head = String::from_utf8_lossy(head_end.map_or(chunk, |h| &chunk[..h]));
        let headers: Vec<(String, String)> = head
            .lines()
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
            .collect();
        let header = |n: &str| {
            headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(n))
                .map(|(_, v)| v.clone())
        };
        let disposition = header("content-disposition").unwrap_or_default();
        parts.push(Part {
            name: param(&disposition, "name"),
            filename: param(&disposition, "filename"),
            content_type: header("content-type"),
            body: match head_end {
                None => end..end,
                Some(h) => start + h + sep..end,
            },
            headers,
        });
        pos = next;
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_boundary_parameter() {
        assert_eq!(
            boundary_of(Some("multipart/form-data; boundary=----abc")).as_deref(),
            Some("----abc")
        );
        assert_eq!(
            boundary_of(Some(r#"multipart/mixed; charset=utf-8; boundary="a b""#)).as_deref(),
            Some("a b")
        );
        assert_eq!(boundary_of(Some("application/json")), None);
    }

    #[test]
    fn splits_parts_with_their_headers() {
        let body = "preamble\r\n--XX\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nHello\r\n\
                    --XX\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a \\\"b\\\".txt\"\r\nContent-Type: text/plain\r\n\r\nline1\r\nline2\r\n\
                    --XX--\r\nepilogue";
        let parts = parse_multipart(body.as_bytes(), "XX");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].name.as_deref(), Some("title"));
        assert_eq!(&body[parts[0].body.clone()], "Hello");
        assert_eq!(parts[1].filename.as_deref(), Some("a \"b\".txt"));
        assert_eq!(parts[1].content_type.as_deref(), Some("text/plain"));
        assert_eq!(&body[parts[1].body.clone()], "line1\r\nline2");
    }

    #[test]
    fn keeps_a_truncated_last_part_and_accepts_lf_line_ends() {
        let body = "--b\nContent-Type: text/plain\n\nfirst\n--b\n\nsecond, cut";
        let parts = parse_multipart(body.as_bytes(), "b");
        let bodies: Vec<&str> = parts.iter().map(|p| &body[p.body.clone()]).collect();
        assert_eq!(bodies, ["first", "second, cut"]);
    }
}
