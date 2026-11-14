//! `FileBuf`: a file's text with its spans, edited in place so span ids stay stable.

use super::segment::*;
use super::{
    hash_str, Edit, EditOutcome, FileBuf, IdAllocator, ParaId, Revision, SegmentRules, Span,
    SpanKind,
};
use std::ops::Range;

impl FileBuf {
    pub fn new(text: &str, ids: &mut IdAllocator, rev: Revision) -> FileBuf {
        Self::with_rules(text, ids, rev, SegmentRules::default())
    }

    pub fn with_rules(
        text: &str,
        ids: &mut IdAllocator,
        rev: Revision,
        rules: SegmentRules,
    ) -> FileBuf {
        let mut fb = FileBuf {
            text: text.to_string(),
            line_starts: compute_line_starts(text),
            spans: Vec::new(),
            rules,
        };
        fb.resegment(ids, rev);
        fb
    }

    /// Re-segment the whole buffer (new ids for every span).
    pub fn resegment(&mut self, ids: &mut IdAllocator, rev: Revision) {
        let units = segment(&self.text, &self.rules);
        self.spans = units
            .into_iter()
            .map(|(range, kind)| {
                let hash = hash_str(&self.text[range.clone()]);
                Span {
                    id: ids.next_id(),
                    range,
                    kind,
                    hash,
                    last_revision: rev,
                }
            })
            .collect();
    }

    pub fn line_of(&self, byte: usize) -> usize {
        match self.line_starts.binary_search(&byte) {
            Ok(i) => i + 1,
            Err(i) => i,
        }
    }
    /// 1-based inclusive line range of a span.
    pub fn line_range(&self, span: &Span) -> (i64, i64) {
        let end = span.range.end.saturating_sub(1).max(span.range.start);
        (
            self.line_of(span.range.start) as i64,
            self.line_of(end) as i64,
        )
    }
    pub fn span_at(&self, byte: usize) -> Option<&Span> {
        self.spans
            .iter()
            .find(|s| s.range.start <= byte && byte <= s.range.end)
    }
    pub fn span(&self, id: ParaId) -> Option<&Span> {
        self.spans.iter().find(|s| s.id == id)
    }
    pub fn span_text(&self, id: ParaId) -> Option<&str> {
        self.span(id).map(|s| &self.text[s.range.clone()])
    }

    /// Apply a byte-range replacement, re-segment, and keep ids stable where the content did not
    /// change (before the edit by position, after it by content hash); the edited unit keeps its
    /// id when the edit maps one old unit to one new unit.
    pub fn apply(&mut self, edit: &Edit, ids: &mut IdAllocator, rev: Revision) -> EditOutcome {
        let start = edit.start_byte.min(self.text.len());
        let end = edit.end_byte.clamp(start, self.text.len());
        let old_spans = std::mem::take(&mut self.spans);
        let delta = edit.text.len() as i64 - (end - start) as i64;
        // in-place splice: one memmove of the tail instead of a full copy
        self.text.replace_range(start..end, &edit.text);
        // incremental line-start table: drop entries inside the replaced range, shift the rest,
        // insert the newlines of the inserted text
        self.splice_line_starts(edit, start, end, delta);
        // Windowed re-segmentation: body spans are delimited by blank lines (environments are kept
        // whole), so re-segmenting from the start of the span before the edit to the end of the
        // span after it reproduces exactly what a full pass would produce there. Edits touching
        // the preamble, the trailer or no span at all fall back to a full pass.
        let window = Self::resegment_window(&old_spans, edit, start, end);
        // The new units are the old spans before the window (`head` of them), the window's
        // own units (`mid`) and the old spans from `tail_from` on, shifted by the edit; a full
        // pass is all `mid`. Only the window is materialized and the span table is updated in
        // place: a keystroke in a long file copies no table.
        let full = || (0, segment(&self.text, &self.rules), old_spans.len());
        let (mut head, mut mid, mut tail_from) = match window {
            Some((lo, hi)) => {
                let lo_is_preamble = old_spans[lo].kind == SpanKind::Preamble;
                let hi_is_trailer = old_spans[hi].kind == SpanKind::Trailer;
                let win_start = if lo_is_preamble {
                    old_spans[lo].range.end
                } else {
                    old_spans[lo].range.start
                };
                let win_end = (if hi_is_trailer {
                    old_spans[hi].range.start
                } else {
                    old_spans[hi].range.end
                } as i64
                    + delta) as usize;
                let keep_prefix = if lo_is_preamble { lo + 1 } else { lo };
                let mut units = Vec::new();
                if segment_body(&self.text, win_start, win_end, &mut units, &self.rules) {
                    (keep_prefix, units, if hi_is_trailer { hi } else { hi + 1 })
                } else {
                    full()
                }
            }
            None => full(),
        };
        let old_len = old_spans.len();
        let new_len = head + mid.len() + (old_len - tail_from);
        let shift =
            |r: &Range<usize>| (r.start as i64 + delta) as usize..(r.end as i64 + delta) as usize;
        let unit = |i: usize, head: usize, mid: &[(Range<usize>, SpanKind)], tail_from: usize| {
            if i < head {
                (old_spans[i].range.clone(), old_spans[i].kind)
            } else if i < head + mid.len() {
                mid[i - head].clone()
            } else {
                let o = &old_spans[i - head - mid.len() + tail_from];
                (shift(&o.range), o.kind)
            }
        };
        // old spans before the edit whose range came out unchanged are unchanged (a span that
        // merely borders the edit, like the preamble when typing at the top of the first
        // paragraph, is unchanged too when its range is)
        let mut prefix = 0;
        while prefix < old_len
            && prefix < new_len
            && old_spans[prefix].range.end <= start
            && old_spans[prefix].range == unit(prefix, head, &mid, tail_from).0
        {
            prefix += 1;
        }
        // old spans entirely after the edit, matched from the end with the byte delta applied
        let mut suffix = 0;
        while suffix < old_len - prefix && suffix < new_len - prefix {
            let o = &old_spans[old_len - 1 - suffix];
            let n = unit(new_len - 1 - suffix, head, &mid, tail_from);
            if o.range.start >= end && n.0 == shift(&o.range) {
                suffix += 1;
            } else {
                break;
            }
        }
        if prefix < head {
            // not expected: the window starts before the edit, so its head is matched prefix.
            // The in-place update below takes a matched suffix's new range from `mid` or from
            // the span itself, never from the head; without that guarantee, use only `mid`.
            mid = (0..new_len)
                .map(|i| unit(i, head, &mid, tail_from))
                .collect();
            (head, tail_from) = (0, old_len);
        }
        let old_mid = &old_spans[prefix..old_len - suffix];
        let new_mid: Vec<(Range<usize>, SpanKind)> = (prefix..new_len - suffix)
            .map(|i| unit(i, head, &mid, tail_from))
            .collect();
        let (outcome, mid_spans) = self.changed_spans(old_mid, &new_mid, ids, rev);
        // the matched prefix stays as it is; the matched suffix keeps ids, hashes and revisions
        // with the new ranges and kinds
        let mut spans = old_spans;
        for k in 0..suffix {
            let (i, j) = (old_len - suffix + k, new_len - suffix + k);
            let sp = &mut spans[i];
            if j >= head + mid.len() {
                sp.range = shift(&sp.range);
            } else {
                (sp.range, sp.kind) = mid[j - head].clone();
            }
        }
        spans.splice(prefix..old_len - suffix, mid_spans);
        self.spans = spans;
        outcome
    }

    /// The line-start table after the text was spliced: entries inside the replaced range go,
    /// the newlines of the inserted text come in, the tail shifts by `delta`.
    fn splice_line_starts(&mut self, edit: &Edit, start: usize, end: usize, delta: i64) {
        let first_after = self.line_starts.partition_point(|&p| p <= start);
        let last_removed = self.line_starts.partition_point(|&p| p <= end);
        let inserted: Vec<usize> = edit
            .text
            .bytes()
            .enumerate()
            .filter(|(_, b)| *b == b'\n')
            .map(|(i, _)| start + i + 1)
            .collect();
        let shifted_from = first_after + inserted.len();
        // in place: a keystroke without a newline moves no entry, it only shifts the tail
        self.line_starts.splice(first_after..last_removed, inserted);
        if delta != 0 {
            for p in &mut self.line_starts[shifted_from..] {
                *p = (*p as i64 + delta) as usize;
            }
        }
        debug_assert_eq!(self.line_starts, compute_line_starts(&self.text));
    }

    /// The old spans (first and last index) that bound what an edit of `start..end` can change,
    /// or None when the whole file must be segmented again.
    fn resegment_window(
        old_spans: &[Span],
        edit: &Edit,
        start: usize,
        end: usize,
    ) -> Option<(usize, usize)> {
        // spans touched by the edit, or (for an edit in a gap between spans) its neighbours
        let touches = |sp: &Span| sp.range.start <= end && start <= sp.range.end;
        let first_touched = old_spans
            .iter()
            .position(touches)
            .or_else(|| old_spans.iter().rposition(|sp| sp.range.end <= start));
        let last_touched = old_spans
            .iter()
            .rposition(touches)
            .or_else(|| old_spans.iter().position(|sp| sp.range.start >= end));
        // The window is bounded by the neighbouring spans; when a neighbour is the preamble or
        // the trailer the window starts (ends) at the body boundary instead, which is fixed.
        let window: Option<(usize, usize)> = match (first_touched, last_touched) {
            (Some(a), Some(b)) if a <= b && a > 0 && b + 1 < old_spans.len() => {
                let (lo, hi) = (a - 1, b + 1);
                let lo_ok = matches!(
                    old_spans[lo].kind,
                    SpanKind::Body | SpanKind::Heading | SpanKind::Env | SpanKind::Preamble
                );
                let hi_ok = matches!(
                    old_spans[hi].kind,
                    SpanKind::Body | SpanKind::Heading | SpanKind::Env | SpanKind::Trailer
                );
                let mid_ok = old_spans[a..=b].iter().all(|sp| {
                    matches!(sp.kind, SpanKind::Body | SpanKind::Heading | SpanKind::Env)
                });
                if lo_ok
                    && hi_ok
                    && mid_ok
                    && !edit.text.contains("\\begin{document}")
                    && !edit.text.contains("\\end{document}")
                {
                    Some((lo, hi))
                } else {
                    None
                }
            }
            _ => None,
        };
        window
    }

    /// The spans that replace `old_mid`, and what the edit did to them: one old span becoming
    /// one new span keeps its id (touched); anything else is a boundary change (removed, added).
    fn changed_spans(
        &self,
        old_mid: &[Span],
        new_mid: &[(Range<usize>, SpanKind)],
        ids: &mut IdAllocator,
        rev: Revision,
    ) -> (EditOutcome, Vec<Span>) {
        let mut outcome = EditOutcome::default();
        let mut mid_spans = Vec::new();
        if old_mid.len() == 1 && new_mid.len() == 1 {
            let o = &old_mid[0];
            let (range, kind) = new_mid[0].clone();
            let hash = hash_str(&self.text[range.clone()]);
            outcome.preamble_changed |= o.kind == SpanKind::Preamble || kind == SpanKind::Preamble;
            outcome.touched.push(o.id);
            mid_spans.push(Span {
                id: o.id,
                range,
                kind,
                hash,
                last_revision: rev,
            });
        } else {
            // A boundary change (split, merge, a paragraph typed fresh). The first new span keeps
            // the id of the first old span when it starts at the same byte with the same kind:
            // its engine context and its first-row placement stay valid (same predecessor, same
            // start). Every other new span gets a fresh id and borrows a context (session).
            let reused = match (old_mid.first(), new_mid.first()) {
                (Some(o), Some((range, kind)))
                    if o.range.start == range.start
                        && o.kind == *kind
                        && matches!(kind, SpanKind::Body | SpanKind::Heading | SpanKind::Env) =>
                {
                    Some(o.id)
                }
                _ => None,
            };
            for o in old_mid {
                if Some(o.id) == reused {
                    continue;
                }
                outcome.removed.push(o.id);
                outcome.preamble_changed |= o.kind == SpanKind::Preamble;
            }
            for (k, (range, kind)) in new_mid.iter().enumerate() {
                let hash = hash_str(&self.text[range.clone()]);
                let id = match (k, reused) {
                    (0, Some(id)) => {
                        outcome.touched.push(id);
                        id
                    }
                    _ => {
                        let id = ids.next_id();
                        outcome.added.push(id);
                        id
                    }
                };
                outcome.preamble_changed |= *kind == SpanKind::Preamble;
                mid_spans.push(Span {
                    id,
                    range: range.clone(),
                    kind: *kind,
                    hash,
                    last_revision: rev,
                });
            }
        }
        (outcome, mid_spans)
    }
}
