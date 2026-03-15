//! Full PageLabel resolver — physical ↔ logical page mapping.
//!
//! The PDF standard (section 12.4.2) allows a `PageLabels` number tree that
//! maps ranges of physical pages to labelling rules.  This module parses the
//! full label specification (style, prefix, logical start) and provides
//! bidirectional resolution between physical indices and human-readable labels.
//!
//! # Fallback chapter detection
//!
//! When `/Outlines` are unavailable, the resolver can also produce chapter
//! boundaries by treating each PageLabel range transition as a potential
//! chapter boundary.

use std::collections::BTreeMap;

use anyhow::{bail, Context, Result};
use lopdf::{Document, Object};

use super::pdf_string_to_string_opt;

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

/// Numbering style for a PageLabel range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelStyle {
    /// Arabic decimal numbers: 1, 2, 3, …
    Decimal,
    /// Lowercase Roman numerals: i, ii, iii, …
    LowerRoman,
    /// Uppercase Roman numerals: I, II, III, …
    UpperRoman,
    /// Lowercase alphabetic: a, b, c, …
    LowerAlpha,
    /// Uppercase alphabetic: A, B, C, …
    UpperAlpha,
}

/// A single PageLabel range as defined by PDF spec.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PageLabelRange {
    /// 0-based physical page index where this labelling rule begins.
    pub physical_start: u32,
    /// Numbering style, or `None` if the range has no numeric portion.
    pub style: Option<LabelStyle>,
    /// Optional prefix string prepended to the numeric portion.
    pub prefix: String,
    /// The logical starting number for this range (defaults to 1).
    pub logical_start: u32,
}

/// Resolver that maps between physical (0-based) and logical page identifiers.
#[allow(dead_code)]
pub struct PageLabelResolver {
    /// Sorted by `physical_start` ascending.
    ranges: Vec<PageLabelRange>,
    /// Total pages in the document.
    total_pages: u32,
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

impl PageLabelResolver {
    /// Parse `/PageLabels` from a lopdf Document.
    pub fn from_document(doc: &Document) -> Result<Self> {
        let catalog = doc.catalog()?;
        let total_pages = doc.get_pages().len() as u32;

        let labels_obj = catalog
            .get(b"PageLabels")
            .context("no /PageLabels in catalog")?;

        let nums_array = Self::extract_nums_array(doc, labels_obj)?;
        let ranges = Self::parse_nums_array(&nums_array)?;

        if ranges.is_empty() {
            bail!("no usable entries in /PageLabels");
        }

        Ok(Self {
            ranges,
            total_pages,
        })
    }

    fn extract_nums_array(doc: &Document, labels_obj: &Object) -> Result<Vec<Object>> {
        match labels_obj {
            Object::Dictionary(d) => d
                .get(b"Nums")
                .context("no Nums in PageLabels")?
                .as_array()
                .context("Nums is not an array")
                .cloned(),
            Object::Array(a) => Ok(a.clone()),
            Object::Reference(r) => {
                let inner = doc.get_object(*r)?;
                Self::extract_nums_array(doc, inner)
            }
            _ => bail!("unexpected PageLabels type"),
        }
    }

    fn parse_nums_array(nums: &[Object]) -> Result<Vec<PageLabelRange>> {
        let mut ranges = Vec::new();
        let mut i = 0;

        while i + 1 < nums.len() {
            let start_page = match &nums[i] {
                Object::Integer(n) => *n as u32,
                _ => {
                    i += 2;
                    continue;
                }
            };

            let (style, prefix, logical_start) = match &nums[i + 1] {
                Object::Dictionary(d) => {
                    let style = d.get(b"S").ok().and_then(Self::parse_style);
                    let prefix = d
                        .get(b"P")
                        .ok()
                        .and_then(pdf_string_to_string_opt)
                        .unwrap_or_default();
                    let logical_start = d
                        .get(b"St")
                        .ok()
                        .and_then(|o| match o {
                            Object::Integer(n) => Some(*n as u32),
                            _ => None,
                        })
                        .unwrap_or(1);
                    (style, prefix, logical_start)
                }
                _ => (None, String::new(), 1),
            };

            ranges.push(PageLabelRange {
                physical_start: start_page,
                style,
                prefix,
                logical_start,
            });

            i += 2;
        }

        Ok(ranges)
    }

    fn parse_style(obj: &Object) -> Option<LabelStyle> {
        let name = match obj {
            Object::Name(bytes) => String::from_utf8_lossy(bytes).to_string(),
            _ => return None,
        };
        match name.as_str() {
            "D" => Some(LabelStyle::Decimal),
            "r" => Some(LabelStyle::LowerRoman),
            "R" => Some(LabelStyle::UpperRoman),
            "a" => Some(LabelStyle::LowerAlpha),
            "A" => Some(LabelStyle::UpperAlpha),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Resolution methods
// ---------------------------------------------------------------------------

#[allow(dead_code)]
impl PageLabelResolver {
    /// Given a 0-based physical page index, return the human-readable label.
    pub fn physical_to_label(&self, physical: u32) -> String {
        let range = self.range_for_physical(physical);

        let offset = physical - range.physical_start;
        let logical_num = range.logical_start + offset;

        let numeric_part = match range.style {
            Some(LabelStyle::Decimal) => format!("{logical_num}"),
            Some(LabelStyle::LowerRoman) => to_roman(logical_num).to_lowercase(),
            Some(LabelStyle::UpperRoman) => to_roman(logical_num),
            Some(LabelStyle::LowerAlpha) => to_alpha(logical_num).to_lowercase(),
            Some(LabelStyle::UpperAlpha) => to_alpha(logical_num),
            None => String::new(),
        };

        format!("{}{}", range.prefix, numeric_part)
    }

    /// Given a human-readable label string, return the 0-based physical page.
    /// Returns `None` if the label is ambiguous or not found.
    pub fn label_to_physical(&self, label: &str) -> Option<u32> {
        for range in &self.ranges {
            // Check if the label starts with this range's prefix.
            if !label.starts_with(&range.prefix) {
                continue;
            }

            let numeric_str = &label[range.prefix.len()..];
            let logical_num = match range.style {
                Some(LabelStyle::Decimal) => match numeric_str.parse::<u32>().ok() {
                    Some(n) => n,
                    None => continue,
                },
                Some(LabelStyle::LowerRoman) => match from_roman(&numeric_str.to_uppercase()) {
                    Some(n) => n,
                    None => continue,
                },
                Some(LabelStyle::UpperRoman) => match from_roman(numeric_str) {
                    Some(n) => n,
                    None => continue,
                },
                Some(LabelStyle::LowerAlpha) | Some(LabelStyle::UpperAlpha) => {
                    match from_alpha(&numeric_str.to_uppercase()) {
                        Some(n) => n,
                        None => continue,
                    }
                }
                None => {
                    if numeric_str.is_empty() {
                        range.logical_start
                    } else {
                        continue;
                    }
                }
            };

            if logical_num < range.logical_start {
                continue;
            }

            let offset = logical_num - range.logical_start;
            let physical = range.physical_start + offset;

            // Verify this physical page falls within this range.
            let range_end = self.range_end(range);
            if physical <= range_end {
                return Some(physical);
            }
        }
        None
    }

    /// Return the 0-based physical page index where Arabic "page 1" begins.
    pub fn content_start(&self) -> Option<u32> {
        self.ranges
            .iter()
            .find(|r| r.style == Some(LabelStyle::Decimal) && r.logical_start == 1)
            .map(|r| r.physical_start)
    }

    /// Compute the offset: physical_index - logical_page_number
    /// for the Arabic-numbered content section.
    pub fn arabic_offset(&self) -> Option<i32> {
        self.content_start().map(|start| start as i32 - 1)
    }

    /// Use PageLabel ranges as chapter boundary hints.
    pub fn as_chapter_boundaries(&self) -> BTreeMap<u32, String> {
        self.ranges
            .iter()
            .map(|r| {
                let label = if !r.prefix.is_empty() {
                    r.prefix.clone()
                } else {
                    format!("Section starting at page {}", r.physical_start)
                };
                (r.physical_start, label)
            })
            .collect()
    }

    /// Find the range that covers the given physical page.
    fn range_for_physical(&self, physical: u32) -> &PageLabelRange {
        // Binary search for the last range with physical_start <= physical.
        let idx = self
            .ranges
            .partition_point(|r| r.physical_start <= physical);
        if idx == 0 {
            &self.ranges[0]
        } else {
            &self.ranges[idx - 1]
        }
    }

    /// Return the last physical page covered by this range.
    fn range_end(&self, range: &PageLabelRange) -> u32 {
        let idx = self
            .ranges
            .iter()
            .position(|r| r.physical_start == range.physical_start)
            .unwrap();
        if idx + 1 < self.ranges.len() {
            self.ranges[idx + 1].physical_start - 1
        } else {
            self.total_pages.saturating_sub(1)
        }
    }
}

// ---------------------------------------------------------------------------
// Roman numeral conversion
// ---------------------------------------------------------------------------

#[allow(dead_code)]
fn to_roman(mut n: u32) -> String {
    const TABLE: &[(u32, &str)] = &[
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut result = String::new();
    for &(value, numeral) in TABLE {
        while n >= value {
            result.push_str(numeral);
            n -= value;
        }
    }
    result
}

#[allow(dead_code)]
pub(crate) fn from_roman(s: &str) -> Option<u32> {
    let roman_val = |c: char| -> Option<u32> {
        match c {
            'I' => Some(1),
            'V' => Some(5),
            'X' => Some(10),
            'L' => Some(50),
            'C' => Some(100),
            'D' => Some(500),
            'M' => Some(1000),
            _ => None,
        }
    };

    let mut total: u32 = 0;
    let mut prev: u32 = 0;

    for c in s.chars().rev() {
        let val = roman_val(c)?;
        if val < prev {
            total = total.checked_sub(val)?;
        } else {
            total = total.checked_add(val)?;
        }
        prev = val;
    }

    if total == 0 {
        None
    } else {
        Some(total)
    }
}

// ---------------------------------------------------------------------------
// Alpha conversion
// ---------------------------------------------------------------------------

#[allow(dead_code)]
fn to_alpha(n: u32) -> String {
    if n == 0 {
        return String::new();
    }
    let c = ((n - 1) % 26) as u8 + b'A';
    String::from(c as char)
}

#[allow(dead_code)]
fn from_alpha(s: &str) -> Option<u32> {
    if s.len() != 1 {
        return None;
    }
    let c = s.chars().next()?;
    if !c.is_ascii_alphabetic() {
        return None;
    }
    Some((c.to_ascii_uppercase() as u32) - ('A' as u32) + 1)
}

// ---------------------------------------------------------------------------
// Standalone chapter extraction (backward-compatible API)
// ---------------------------------------------------------------------------

/// Parse `/PageLabels` and treat each range start as a chapter boundary.
///
/// This is the original simpler API preserved for the chunking engine's
/// fallback chain.
pub fn extract_chapters_from_page_labels(doc: &Document) -> Result<BTreeMap<u32, String>> {
    match PageLabelResolver::from_document(doc) {
        Ok(resolver) => Ok(resolver.as_chapter_boundaries()),
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roman_numerals_round_trip() {
        for n in 1..=50 {
            let roman = to_roman(n);
            let back = from_roman(&roman).unwrap();
            assert_eq!(n, back, "failed for {n} → {roman}");
        }
    }

    #[test]
    fn roman_known_values() {
        assert_eq!(to_roman(1), "I");
        assert_eq!(to_roman(4), "IV");
        assert_eq!(to_roman(9), "IX");
        assert_eq!(to_roman(14), "XIV");
        assert_eq!(to_roman(42), "XLII");
    }

    #[test]
    fn alpha_round_trip() {
        for n in 1..=26 {
            let alpha = to_alpha(n);
            let back = from_alpha(&alpha).unwrap();
            assert_eq!(n, back);
        }
    }

    #[test]
    fn resolver_physical_to_label() {
        // Simulate a textbook: pages 0-11 roman, pages 12+ arabic
        let resolver = PageLabelResolver {
            ranges: vec![
                PageLabelRange {
                    physical_start: 0,
                    style: Some(LabelStyle::LowerRoman),
                    prefix: String::new(),
                    logical_start: 1,
                },
                PageLabelRange {
                    physical_start: 12,
                    style: Some(LabelStyle::Decimal),
                    prefix: String::new(),
                    logical_start: 1,
                },
            ],
            total_pages: 200,
        };

        assert_eq!(resolver.physical_to_label(0), "i");
        assert_eq!(resolver.physical_to_label(3), "iv");
        assert_eq!(resolver.physical_to_label(11), "xii");
        assert_eq!(resolver.physical_to_label(12), "1");
        assert_eq!(resolver.physical_to_label(13), "2");
        assert_eq!(resolver.physical_to_label(111), "100");
    }

    #[test]
    fn resolver_label_to_physical() {
        let resolver = PageLabelResolver {
            ranges: vec![
                PageLabelRange {
                    physical_start: 0,
                    style: Some(LabelStyle::LowerRoman),
                    prefix: String::new(),
                    logical_start: 1,
                },
                PageLabelRange {
                    physical_start: 12,
                    style: Some(LabelStyle::Decimal),
                    prefix: String::new(),
                    logical_start: 1,
                },
            ],
            total_pages: 200,
        };

        assert_eq!(resolver.label_to_physical("iv"), Some(3));
        assert_eq!(resolver.label_to_physical("xii"), Some(11));
        assert_eq!(resolver.label_to_physical("1"), Some(12));
        assert_eq!(resolver.label_to_physical("100"), Some(111));
    }

    #[test]
    fn resolver_content_start() {
        let resolver = PageLabelResolver {
            ranges: vec![
                PageLabelRange {
                    physical_start: 0,
                    style: Some(LabelStyle::UpperRoman),
                    prefix: String::new(),
                    logical_start: 1,
                },
                PageLabelRange {
                    physical_start: 8,
                    style: Some(LabelStyle::Decimal),
                    prefix: String::new(),
                    logical_start: 1,
                },
            ],
            total_pages: 100,
        };

        assert_eq!(resolver.content_start(), Some(8));
        assert_eq!(resolver.arabic_offset(), Some(7));
    }

    #[test]
    fn resolver_with_prefix() {
        let resolver = PageLabelResolver {
            ranges: vec![
                PageLabelRange {
                    physical_start: 0,
                    style: Some(LabelStyle::Decimal),
                    prefix: "A-".to_string(),
                    logical_start: 1,
                },
                PageLabelRange {
                    physical_start: 5,
                    style: Some(LabelStyle::Decimal),
                    prefix: "B-".to_string(),
                    logical_start: 1,
                },
            ],
            total_pages: 10,
        };

        assert_eq!(resolver.physical_to_label(0), "A-1");
        assert_eq!(resolver.physical_to_label(4), "A-5");
        assert_eq!(resolver.physical_to_label(5), "B-1");
        assert_eq!(resolver.physical_to_label(7), "B-3");

        assert_eq!(resolver.label_to_physical("A-3"), Some(2));
        assert_eq!(resolver.label_to_physical("B-2"), Some(6));
    }

    #[test]
    fn chapter_boundaries() {
        let resolver = PageLabelResolver {
            ranges: vec![
                PageLabelRange {
                    physical_start: 0,
                    style: Some(LabelStyle::LowerRoman),
                    prefix: "Preface".to_string(),
                    logical_start: 1,
                },
                PageLabelRange {
                    physical_start: 10,
                    style: Some(LabelStyle::Decimal),
                    prefix: "Chapter ".to_string(),
                    logical_start: 1,
                },
            ],
            total_pages: 50,
        };

        let boundaries = resolver.as_chapter_boundaries();
        assert_eq!(boundaries.len(), 2);
        assert_eq!(boundaries[&0], "Preface");
        assert_eq!(boundaries[&10], "Chapter ");
    }
}
