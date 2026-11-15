use super::*;
use gpui::{
    AvailableSpace, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, LayoutId,
    Style, TextRun, point, size,
};
use std::{cell::RefCell, rc::Rc};

struct Group {
    runs: Vec<InlineRun>,
    text: SharedString,
    offset: usize,
    word_base: usize,
    code: bool,
}

struct Fragment {
    group: usize,
    range: Range<usize>,
    bounds: Bounds<Pixels>,
}

#[derive(Default)]
struct Plan {
    width: Pixels,
    height: Pixels,
    fragments: Vec<Fragment>,
}

struct Flow {
    groups: Rc<Vec<Group>>,
    id: SharedString,
    tone: Tone,
    fade: Option<crate::reveal::WordFades>,
    font_size: f32,
    selection: Option<TextSelection>,
    children: Vec<AnyElement>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn paragraph(
    runs: &[InlineRun],
    id: SharedString,
    tone: Tone,
    fade: Option<&crate::reveal::WordFades>,
    mut word_base: usize,
    font_size: f32,
    selection: Option<TextSelection>,
) -> AnyElement {
    let mut groups = Vec::new();
    let mut offset = 0;
    let mut start = 0;
    while start < runs.len() {
        let code = runs[start].style.code;
        let end = if code {
            start + 1
        } else {
            (start + 1..runs.len())
                .find(|&index| runs[index].style.code)
                .unwrap_or(runs.len())
        };
        let text: String = runs[start..end]
            .iter()
            .map(|run| run.text.as_str())
            .collect();
        groups.push(Group {
            runs: runs[start..end]
                .iter()
                .cloned()
                .map(|mut run| {
                    run.style.task = None;
                    run
                })
                .collect(),
            text: text.clone().into(),
            offset,
            word_base,
            code,
        });
        word_base += eligible_words(&runs[start..end]);
        offset += text.len();
        start = end;
    }
    div()
        .id(id.clone())
        .role(Role::Label)
        .aria_label(runs.iter().map(|run| run.text.as_str()).collect::<String>())
        .w_full()
        .text_size(px(font_size))
        .line_height(px(24.))
        .child(Flow {
            groups: Rc::new(groups),
            id,
            tone,
            fade: fade.cloned(),
            font_size,
            selection,
            children: Vec::new(),
        })
        .into_any_element()
}

fn eligible_words(runs: &[InlineRun]) -> usize {
    runs.iter()
        .filter(|run| !run.style.code && !run.style.math && run.style.link.is_none())
        .map(|run| run.text.split_whitespace().count())
        .sum()
}

fn slice_runs(group: &Group, range: Range<usize>) -> (Vec<InlineRun>, usize) {
    let mut runs = Vec::new();
    let mut offset = 0;
    let mut word_base = group.word_base;
    for run in &group.runs {
        let start = range.start.saturating_sub(offset).min(run.text.len());
        let end = range.end.saturating_sub(offset).min(run.text.len());
        if !run.style.math && run.style.link.is_none() {
            let prefix = &run.text[..start];
            let continuation = start < run.text.len()
                && prefix.chars().last().is_some_and(|ch| !ch.is_whitespace())
                && run.text[start..]
                    .chars()
                    .next()
                    .is_some_and(|ch| !ch.is_whitespace());
            word_base += prefix.split_whitespace().count() - usize::from(continuation);
        }
        if start < end {
            runs.push(InlineRun {
                text: run.text[start..end].into(),
                style: run.style.clone(),
            });
        }
        offset += run.text.len();
        if offset >= range.end {
            break;
        }
    }
    (runs, word_base)
}

fn text_runs(group: &Group, style: &gpui::TextStyle, tone: Tone) -> Vec<TextRun> {
    group
        .runs
        .iter()
        .map(|run| {
            let mut text_run = style.to_run(run.text.len());
            if !group.code && run.style.bold {
                text_run.font.weight = FontWeight::SEMIBOLD;
            }
            if !group.code && run.style.italic {
                text_run.font.style = FontStyle::Italic;
            }
            if group.code {
                text_run.font.family = crate::fonts::mono().into();
            }
            text_run.color = if group.code {
                content(1.)
            } else if run.style.link.is_some() {
                accent_color()
            } else if run.style.bold || run.style.italic {
                content(tone.strong())
            } else {
                content(tone.prose())
            };
            if let Some(search) = run.style.search {
                text_run.background_color = Some(search::search_background(search));
            }
            if run.style.strikethrough {
                text_run.strikethrough = Some(StrikethroughStyle {
                    thickness: px(1.),
                    color: None,
                });
            }
            text_run
        })
        .collect()
}

// TextLayout rounds logical widths before Taffy rounds device-pixel widths.
fn text_width(width: Pixels, window: &Window) -> Pixels {
    let scale = window.scale_factor();
    px((f32::from(width.ceil()) * scale).ceil() / scale)
}

// Keep GPUI's per-line accumulation order before device-pixel rounding.
fn block_height(
    line_height: Pixels,
    lines: impl IntoIterator<Item = usize>,
    window: &Window,
) -> Pixels {
    let height = lines
        .into_iter()
        .fold(px(0.), |height, count| height + line_height * count);
    let scale = window.scale_factor();
    px((f32::from(height) * scale).ceil() / scale)
}

fn code_overhead(text: &str, window: &Window) -> Pixels {
    let padding = window.pixel_snap(px(6.)) * 2.;
    if is_file_chip(text) {
        padding + window.pixel_snap(px(14.)) + window.pixel_snap(window.rem_size() * 0.25)
    } else {
        padding
    }
}

fn measure(
    groups: &[Group],
    width: Pixels,
    font_size: f32,
    style: &gpui::TextStyle,
    tone: Tone,
    window: &Window,
) -> Plan {
    let mut plan = Plan {
        width,
        ..Default::default()
    };
    let mut x = px(0.);
    let mut y = px(0.);
    let line_height = window.pixel_snap(px(24.));
    let row_height = block_height(line_height, [1], window);
    let mut height = row_height;
    for (index, group) in groups.iter().enumerate() {
        let runs = text_runs(group, style, tone);
        if group.code {
            let overhead = code_overhead(&group.text, window);
            let code_size = px(font_size * 0.8);
            let natural = window
                .text_system()
                .shape_text(group.text.clone(), code_size, &runs, None, None)
                .unwrap();
            let natural_width = natural
                .iter()
                .map(|line| line.width())
                .fold(px(0.), Pixels::max);
            let chip_width = (text_width(natural_width, window) + overhead).min(width);
            if x > px(0.) && x + chip_width > width {
                y += height;
                x = px(0.);
                height = row_height;
            }
            let wrapped = window
                .text_system()
                .shape_text(
                    group.text.clone(),
                    code_size,
                    &runs,
                    Some((chip_width - overhead).max(px(1.))),
                    None,
                )
                .unwrap();
            let chip_height = block_height(
                line_height,
                wrapped.iter().map(|line| line.wrap_boundaries.len() + 1),
                window,
            );
            plan.fragments.push(Fragment {
                group: index,
                range: 0..group.text.len(),
                bounds: Bounds::new(point(x, y), size(chip_width, chip_height)),
            });
            x += chip_width;
            height = height.max(chip_height);
            continue;
        }
        let mut cursor = 0;
        while cursor < group.text.len() {
            if x >= width {
                y += height;
                x = px(0.);
                height = row_height;
            }
            let (sliced, _) = slice_runs(group, cursor..group.text.len());
            let temporary = Group {
                text: group.text[cursor..].to_owned().into(),
                runs: sliced,
                offset: 0,
                word_base: 0,
                code: false,
            };
            let remaining = (width - x).max(px(1.));
            let shaped = window
                .text_system()
                .shape_text(
                    temporary.text.clone(),
                    px(font_size),
                    &text_runs(&temporary, style, tone),
                    Some(remaining),
                    None,
                )
                .unwrap();
            let first = &shaped[0];
            let lines = shaped
                .iter()
                .map(|line| line.wrap_boundaries.len() + 1)
                .sum::<usize>();
            if x == px(0.) || lines == 1 {
                let last = shaped.last().unwrap();
                let last_start = last.wrap_boundaries.last().map_or(0, |boundary| {
                    last.unwrapped_layout.runs[boundary.run_ix].glyphs[boundary.glyph_ix].index
                });
                let last_width = text_width(
                    last.unwrapped_layout.width - last.unwrapped_layout.x_for_index(last_start),
                    window,
                )
                .min(remaining);
                let fragment_width = if lines > 1 { remaining } else { last_width };
                let fragment_height = block_height(
                    line_height,
                    shaped.iter().map(|line| line.wrap_boundaries.len() + 1),
                    window,
                );
                let mut prefix_lines = lines - 1;
                let prefix_height = block_height(
                    line_height,
                    shaped.iter().map(|line| {
                        let count = (line.wrap_boundaries.len() + 1).min(prefix_lines);
                        prefix_lines -= count;
                        count
                    }),
                    window,
                );
                let split_last = lines > 1 && index + 1 < groups.len();
                // Separate the last line so selection bounds cannot cover the next chip.
                if split_last {
                    let logical_start: usize = shaped[..shaped.len() - 1]
                        .iter()
                        .map(|line| line.text.len() + 1)
                        .sum();
                    let tail_start = cursor + logical_start + last_start;
                    let prefix_end = tail_start
                        - usize::from(
                            group.text.as_bytes().get(tail_start.wrapping_sub(1)) == Some(&b'\n'),
                        );
                    if cursor < prefix_end {
                        plan.fragments.push(Fragment {
                            group: index,
                            range: cursor..prefix_end,
                            bounds: Bounds::new(point(x, y), size(remaining, prefix_height)),
                        });
                    }
                    if tail_start < group.text.len() {
                        plan.fragments.push(Fragment {
                            group: index,
                            range: tail_start..group.text.len(),
                            bounds: Bounds::new(
                                point(px(0.), y + prefix_height),
                                size(last_width, row_height),
                            ),
                        });
                    }
                } else {
                    plan.fragments.push(Fragment {
                        group: index,
                        range: cursor..group.text.len(),
                        bounds: Bounds::new(point(x, y), size(fragment_width, fragment_height)),
                    });
                }
                y += if split_last {
                    prefix_height
                } else {
                    fragment_height - row_height
                };
                x = if lines > 1 {
                    last_width
                } else {
                    x + last_width
                };
                height = if lines > 1 { row_height } else { height };
                break;
            }
            let count = first
                .wrap_boundaries
                .first()
                .map_or(first.text.len(), |boundary| {
                    first.unwrapped_layout.runs[boundary.run_ix].glyphs[boundary.glyph_ix].index
                });
            if count > 0
                && count < first.text.len()
                && !temporary.text[..count].contains(char::is_whitespace)
            {
                y += height;
                x = px(0.);
                height = row_height;
                continue;
            }
            if count > 0 {
                let fragment_width =
                    text_width(first.unwrapped_layout.x_for_index(count), window).min(remaining);
                plan.fragments.push(Fragment {
                    group: index,
                    range: cursor..cursor + count,
                    bounds: Bounds::new(point(x, y), size(fragment_width, row_height)),
                });
            }
            cursor += count;
            if temporary.text.as_bytes().get(count) == Some(&b'\n') {
                cursor += 1;
            }
            y += height;
            x = px(0.);
            height = row_height;
        }
    }
    plan.height = y + height;
    plan
}

impl IntoElement for Flow {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for Flow {
    type RequestLayoutState = Rc<RefCell<Plan>>;
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        _: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let plan = Rc::new(RefCell::new(Plan::default()));
        let measured = plan.clone();
        let groups = self.groups.clone();
        let font_size = self.font_size;
        let text_style = window.text_style();
        let tone = self.tone;
        let layout =
            window.request_measured_layout(Style::default(), move |known, available, window, _| {
                let width = known
                    .width
                    .unwrap_or(match available.width {
                        AvailableSpace::Definite(width) => width,
                        AvailableSpace::MinContent => px(1.),
                        AvailableSpace::MaxContent => groups
                            .iter()
                            .map(|group| {
                                let font = px(if group.code {
                                    font_size * 0.8
                                } else {
                                    font_size
                                });
                                let shaped = window
                                    .text_system()
                                    .shape_text(
                                        group.text.clone(),
                                        font,
                                        &text_runs(group, &text_style, tone),
                                        None,
                                        None,
                                    )
                                    .unwrap();
                                shaped
                                    .iter()
                                    .map(|line| line.width())
                                    .fold(px(0.), Pixels::max)
                                    + if group.code {
                                        code_overhead(&group.text, window)
                                    } else {
                                        px(0.)
                                    }
                            })
                            .sum(),
                    })
                    .max(px(1.));
                let scale = window.scale_factor();
                let width = px((f32::from(width) * scale).floor().max(1.) / scale);
                if measured.borrow().width == width && measured.borrow().height > px(0.) {
                    let cached = measured.borrow();
                    return size(cached.width, cached.height);
                }
                let next = measure(&groups, width, font_size, &text_style, tone, window);
                let size = size(next.width, next.height);
                *measured.borrow_mut() = next;
                size
            });
        (layout, plan)
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        plan: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let plan = plan.borrow();
        self.children.clear();
        for (index, fragment) in plan.fragments.iter().enumerate() {
            let group = &self.groups[fragment.group];
            let selected = self
                .selection
                .as_ref()
                .map(|selection| selection.offset(group.offset + fragment.range.start));
            let mut child = if group.code {
                code_chip(
                    &group.runs[0],
                    &self.id,
                    fragment.group,
                    self.font_size,
                    selected,
                )
            } else {
                let (runs, word_base) = slice_runs(group, fragment.range.clone());
                let mut prepared = cache::PreparedInline::new(&runs, false);
                prepared.fonts.clear();
                inline_selected(
                    &runs,
                    format!("{}-flow-{index}", self.id).into(),
                    self.tone,
                    self.fade.as_ref(),
                    Instant::now(),
                    word_base,
                    self.font_size,
                    selected,
                    Some(&prepared),
                    true,
                )
            };
            let measured = child.layout_as_root(
                size(
                    AvailableSpace::Definite(fragment.bounds.size.width),
                    AvailableSpace::MinContent,
                ),
                window,
                cx,
            );
            #[cfg(test)]
            assert!(
                measured.height <= fragment.bounds.size.height + px(0.1),
                "fragment {:?}: measured {measured:?}, planned {:?}",
                fragment.range,
                fragment.bounds
            );
            let _ = measured;
            child.prepaint_at(bounds.origin + fragment.bounds.origin, window, cx);
            self.children.push(child);
        }
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        for child in &mut self.children {
            child.paint(window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragment_ranges_keep_styles_bytes_and_global_word_indices() {
        let runs = vec![
            InlineRun {
                text: "one café ".into(),
                style: Default::default(),
            },
            InlineRun {
                text: "two ".into(),
                style: crate::markdown_model::InlineStyle {
                    bold: true,
                    ..Default::default()
                },
            },
            InlineRun {
                text: "read here ".into(),
                style: crate::markdown_model::InlineStyle {
                    link: Some("https://example.com".into()),
                    ..Default::default()
                },
            },
            InlineRun {
                text: "three 👩‍💻".into(),
                style: Default::default(),
            },
        ];
        let group = Group {
            text: runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>()
                .into(),
            runs,
            offset: 7,
            word_base: 50,
            code: false,
        };
        let original = cache::PreparedInline::new(&group.runs, false);
        for (start, _) in group.text.char_indices() {
            let (sliced, word_base) = slice_runs(&group, start..group.text.len());
            let prepared = cache::PreparedInline::new(&sliced, false);
            assert_eq!(&*prepared.text, &group.text[start..]);
            for run in &prepared.runs {
                for (range, word) in &run.pieces {
                    if let Some(word) = word {
                        let byte = start + range.start;
                        let original_word = original
                            .runs
                            .iter()
                            .flat_map(|run| &run.pieces)
                            .find_map(|(range, word)| {
                                range.contains(&byte).then_some(*word).flatten()
                            })
                            .unwrap();
                        assert_eq!(word_base + word, group.word_base + original_word);
                    }
                }
            }
        }
    }
}
