//! Small-cap shaping with the same native/synthesized choice as authoritative
//! OOXML measurement. The stored display list and its source text stay intact.

use super::*;
use ooxml_text::caps::{WORD_SMALL_CAPS_ADVANCE_SCALE, uppercase_for_language};

const SMCP: [ShapeFeature; 1] = [ShapeFeature {
    tag: *b"smcp",
    value: 1,
}];

struct Segment {
    text: String,
    font: FontId,
    scale: f32,
    native: bool,
    // Shaped byte offsets mapped to original character indices. Expansions
    // such as ß -> SS must not introduce an extra letter-spacing gap.
    sources: Vec<(usize, usize)>,
}

impl Segment {
    fn source(&self, cluster: u32) -> usize {
        let slot = self
            .sources
            .partition_point(|&(byte, _)| byte <= cluster as usize);
        self.sources[slot.saturating_sub(1)].1
    }
}

fn resolve(store: &FontStore, chain: &[FontId], ch: char) -> Result<FontId, String> {
    store
        .resolve(chain, ch)
        // Match measurement's .notdef fallback rather than silently dropping
        // an unsupported source character.
        .or_else(|| chain.last().copied())
        .ok_or_else(|| "small-caps font chain is empty".to_string())
}

fn supports_smcp(
    store: &FontStore,
    font: FontId,
    ch: char,
    size: f32,
    direction: ShapeDirection,
) -> Result<bool, String> {
    let text = ch.to_string();
    // Measurement probes the substitution without language properties, then
    // supplies the language when shaping the complete segment.
    let plain = ooxml_text::shape_with_direction(store, font, &text, size, &[], direction)
        .map_err(|error| error.to_string())?;
    let caps = ooxml_text::shape_with_direction(store, font, &text, size, &SMCP, direction)
        .map_err(|error| error.to_string())?;
    Ok(!plain
        .iter()
        .map(|g| g.glyph_id)
        .eq(caps.iter().map(|g| g.glyph_id)))
}

fn segments(
    store: &FontStore,
    chain: &[FontId],
    run: &TextRunPrimitive,
    size: f32,
    direction: ShapeDirection,
) -> Result<Vec<Segment>, String> {
    let mut result: Vec<Segment> = Vec::new();
    let mut substitutions = HashMap::new();
    for (source, ch) in run.text.chars().enumerate() {
        let lowercase = !run.all_caps && ch.is_lowercase();
        let mut forms = if run.all_caps || lowercase {
            uppercase_for_language(ch, run.attrs.lang.as_deref())
        } else {
            vec![ch]
        };
        let mut font = resolve(store, chain, forms[0])?;
        let mut native = false;
        let mut scale = if lowercase {
            WORD_SMALL_CAPS_ADVANCE_SCALE
        } else {
            1.0
        };
        if lowercase {
            let original_font = resolve(store, chain, ch)?;
            native = match substitutions.get(&(original_font, ch)) {
                Some(&supported) => supported,
                None => {
                    let supported = supports_smcp(store, original_font, ch, size, direction)?;
                    substitutions.insert((original_font, ch), supported);
                    supported
                }
            };
            if native {
                forms = vec![ch];
                font = original_font;
                scale = 1.0;
            }
        }
        let append = result
            .last()
            .is_some_and(|last| last.font == font && last.scale == scale && last.native == native);
        if !append {
            result.push(Segment {
                text: String::new(),
                font,
                scale,
                native,
                sources: Vec::new(),
            });
        }
        let segment = result.last_mut().expect("a segment was just selected");
        for form in forms {
            segment.sources.push((segment.text.len(), source));
            segment.text.push(form);
        }
    }
    Ok(result)
}

// Charge before creating uppercase strings or per-character plans. A single
// character can expand into multiple glyphs, just as with allCaps.
fn painted_chars(run: &TextRunPrimitive) -> u64 {
    run.text
        .chars()
        .map(|ch| {
            if run.all_caps || ch.is_lowercase() {
                uppercase_for_language(ch, run.attrs.lang.as_deref()).len() as u64
            } else {
                1
            }
        })
        .sum()
}

pub(super) fn paint(
    context: &mut PaintContext<'_, '_>,
    run: &TextRunPrimitive,
    parsed: &ParsedFont,
    color: tiny_skia::Color,
    transform: Transform,
) -> Result<(), String> {
    context.budget.charge_glyphs(painted_chars(run))?;
    let chain = context
        .resources
        .font_chains
        .get(&parsed.chain_key)
        .ok_or_else(|| format!("missing font chain for `{}`", parsed.chain_key))?;
    let direction = if run.rtl == Some(true) {
        ShapeDirection::Rtl
    } else {
        ShapeDirection::Ltr
    };
    let segments = segments(
        context.resources.fonts,
        chain,
        run,
        parsed.spec.size,
        direction,
    )?;
    let letter_spacing = run
        .letter_spacing
        .as_ref()
        .map(number_f32)
        .transpose()?
        .unwrap_or(0.0);
    let word_spacing = run
        .word_spacing
        .as_ref()
        .map(number_f32)
        .transpose()?
        .unwrap_or(0.0);
    let baseline = number_f32(&run.baseline_y)?;
    let mut pen = number_f32(&run.x)?;
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;
    let order: Box<dyn Iterator<Item = usize>> = if run.rtl == Some(true) {
        Box::new((0..segments.len()).rev())
    } else {
        Box::new(0..segments.len())
    };
    for (visual_index, segment_index) in order.enumerate() {
        let segment = &segments[segment_index];
        // Shape at the original size, then scale advances and ink together.
        // This matches measurement, including kerning within synthesized caps.
        let glyphs = shape_with_properties(
            context.resources.fonts,
            segment.font,
            &segment.text,
            parsed.spec.size,
            if segment.native { &SMCP } else { &[] },
            direction,
            run.attrs.lang.as_deref(),
        )
        .map_err(|error| error.to_string())?;
        charge_shaped_excess(context, &segment.text, glyphs.len())?;
        for (index, glyph) in glyphs.iter().enumerate() {
            let placed = PlacedGlyph {
                id: glyph.glyph_id,
                x: f64::from(pen + glyph.x_offset * segment.scale),
                y: f64::from(baseline - glyph.y_offset * segment.scale),
                cluster: glyph.cluster,
                advance: f64::from(glyph.x_advance * segment.scale),
                logical_order: None,
                bidi_level: None,
            };
            paint_placed_glyph(
                context,
                segment.font,
                &placed,
                parsed.spec.size * segment.scale,
                &paint,
                transform,
            )?;
            pen += glyph.x_advance * segment.scale;
            let source = segment.source(glyph.cluster);
            let ends_source = glyphs
                .get(index + 1)
                .is_none_or(|next| segment.source(next.cluster) != source);
            if ends_source {
                let is_space = segment
                    .text
                    .get(glyph.cluster as usize..)
                    .is_some_and(|tail| tail.starts_with(' '));
                if is_space {
                    pen += word_spacing;
                }
                if index + 1 < glyphs.len() || visual_index + 1 < segments.len() {
                    pen += letter_spacing;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
