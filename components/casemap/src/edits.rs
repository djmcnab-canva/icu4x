// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use core::ops::Range;
use writeable::Writeable;

/// A changed source range and its replacement in the case-mapped output.
///
/// Both ranges use UTF-8 byte offsets, relative to the beginning of the input and
/// output of this mapping operation. Their endpoints are character boundaries.
/// An empty destination range represents a deletion. Equal-length ranges can
/// still represent a change, for example `a` to `A`.
///
/// Edits are ordered, non-overlapping, and reported separately for each changed
/// source character. Gaps between edits are unchanged text. Context-sensitive
/// mappings may depend on characters outside the source range of an edit.
/// Within a replacement there is no unique correspondence between positions;
/// applications can choose the start or end of the replacement for a cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CaseMapEdit {
    /// The range replaced in the source string.
    pub source: Range<usize>,
    /// The replacement range in the output string.
    pub destination: Range<usize>,
}

/// A case-mapping result that can also report how the source text changed.
///
/// Returned by the string-mapping methods on [`crate::CaseMapperBorrowed`] and
/// [`crate::TitlecaseMapperBorrowed`]. Ordinary [`Writeable`] methods remain
/// available when edit information is not needed.
///
/// # Examples
///
/// ```
/// use icu::casemap::{CaseMapper, CaseMapWriteable};
/// use icu::locale::langid;
///
/// let (output, edits) = CaseMapper::new()
///     .uppercase("aß!", &langid!("und"))
///     .to_string_with_edits();
/// assert_eq!(output, "ASS!");
/// assert_eq!(edits.len(), 2);
/// assert_eq!(edits[0].source, 0..1);
/// assert_eq!(edits[0].destination, 0..1);
/// assert_eq!(edits[1].source, 1..3);
/// assert_eq!(edits[1].destination, 1..3);
/// ```
pub trait CaseMapWriteable: Writeable {
    /// Writes the mapped text and calls `record_edit` for each change.
    ///
    /// Offsets start at zero even if `sink` already contains text. Unchanged
    /// characters do not produce edits. This method does not allocate.
    ///
    /// Errors from `sink` are propagated immediately. On error, the sink and
    /// callback may have received partial output; only fully written mappings
    /// have their edits reported.
    fn write_to_with_edits<W: fmt::Write + ?Sized>(
        &self,
        sink: &mut W,
        record_edit: impl FnMut(CaseMapEdit),
    ) -> fmt::Result;

    /// Returns the mapped string together with its changes in a single pass.
    fn to_string_with_edits(&self) -> (String, Vec<CaseMapEdit>) {
        let mut output = String::new();
        let mut edits = Vec::new();
        let _ = self.write_to_with_edits(&mut output, |edit| edits.push(edit));
        (output, edits)
    }
}

// Share the mapping algorithm while allowing ordinary Writeable calls to skip
// all edit tracking and comparisons.
pub(crate) trait CaseMapSink {
    type Writer: fmt::Write + ?Sized;

    fn write_mapping(
        &mut self,
        source: char,
        write: impl FnOnce(&mut Self::Writer) -> fmt::Result,
    ) -> fmt::Result;

    fn write_str(&mut self, unchanged: &str) -> fmt::Result;
}

pub(crate) struct PlainSink<'a, W: ?Sized>(pub &'a mut W);

impl<W: fmt::Write + ?Sized> CaseMapSink for PlainSink<'_, W> {
    type Writer = W;

    fn write_mapping(&mut self, _: char, write: impl FnOnce(&mut W) -> fmt::Result) -> fmt::Result {
        write(self.0)
    }

    fn write_str(&mut self, unchanged: &str) -> fmt::Result {
        self.0.write_str(unchanged)
    }
}

pub(crate) struct EditSink<'a, W: ?Sized, F> {
    writer: ComparingWriter<'a, W>,
    record_edit: F,
    source_offset: usize,
    destination_offset: usize,
}

impl<'a, W: fmt::Write + ?Sized, F: FnMut(CaseMapEdit)> EditSink<'a, W, F> {
    pub(crate) fn new(sink: &'a mut W, record_edit: F) -> Self {
        Self {
            writer: ComparingWriter {
                sink,
                source: [0; 4],
                source_len: 0,
                written: 0,
                changed: false,
            },
            record_edit,
            source_offset: 0,
            destination_offset: 0,
        }
    }
}

pub(crate) struct ComparingWriter<'a, W: ?Sized> {
    sink: &'a mut W,
    source: [u8; 4],
    source_len: usize,
    written: usize,
    changed: bool,
}

impl<W: fmt::Write + ?Sized> fmt::Write for ComparingWriter<'_, W> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.sink.write_str(s)?;
        self.changed |= self
            .source
            .get(..self.source_len)
            .and_then(|source| source.get(self.written..))
            .and_then(|remaining| remaining.get(..s.len()))
            != Some(s.as_bytes());
        self.written += s.len();
        Ok(())
    }
}

impl<'a, W: fmt::Write + ?Sized, F: FnMut(CaseMapEdit)> CaseMapSink for EditSink<'a, W, F> {
    type Writer = ComparingWriter<'a, W>;

    fn write_mapping(
        &mut self,
        source: char,
        write: impl FnOnce(&mut Self::Writer) -> fmt::Result,
    ) -> fmt::Result {
        self.writer.source_len = source.encode_utf8(&mut self.writer.source).len();
        self.writer.written = 0;
        self.writer.changed = false;
        write(&mut self.writer)?;
        let source_end = self.source_offset + source.len_utf8();
        let destination_end = self.destination_offset + self.writer.written;
        if self.writer.changed || self.writer.written != source.len_utf8() {
            (self.record_edit)(CaseMapEdit {
                source: self.source_offset..source_end,
                destination: self.destination_offset..destination_end,
            });
        }
        self.source_offset = source_end;
        self.destination_offset = destination_end;
        Ok(())
    }

    fn write_str(&mut self, unchanged: &str) -> fmt::Result {
        self.writer.sink.write_str(unchanged)?;
        self.source_offset += unchanged.len();
        self.destination_offset += unchanged.len();
        Ok(())
    }
}
