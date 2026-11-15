use super::*;
use crate::{markdown_model::InlineStyle, reveal::PacedText};
use gpui::{AvailableSpace, TestAppContext, TextLayout, VisualTestContext, point, size};
use std::{cell::RefCell, time::Duration};

thread_local! {
    static OBSERVED: RefCell<Option<Vec<(TextLayout, usize)>>> = const { RefCell::new(None) };
}

pub(super) fn observe(layout: &TextLayout, highlights: usize) {
    OBSERVED.with_borrow_mut(|observed| {
        if let Some(observed) = observed {
            observed.push((layout.clone(), highlights));
        }
    });
}

pub(super) fn begin_capture() {
    OBSERVED.with_borrow_mut(|observed| *observed = Some(Vec::new()));
}

pub(super) fn finish_capture() -> Vec<TextLayout> {
    OBSERVED
        .with_borrow_mut(|observed| observed.take().unwrap())
        .into_iter()
        .map(|(layout, _)| layout)
        .collect()
}

fn fixture(code: bool) -> Vec<Arc<MarkdownRow>> {
    let mut result = Vec::new();
    let mut word_start = 0;
    for paragraph in 0..6 {
        let mut runs = vec![
            InlineRun {
                text: format!("Paragraph {paragraph} café words for the native response. ")
                    .repeat(6),
                style: InlineStyle::default(),
            },
            InlineRun {
                text: "Bold text ".into(),
                style: InlineStyle {
                    bold: true,
                    ..Default::default()
                },
            },
            InlineRun {
                text: "a link ".into(),
                style: InlineStyle {
                    link: Some("https://example.com".into()),
                    ..Default::default()
                },
            },
            InlineRun {
                text: "italic end".into(),
                style: InlineStyle {
                    italic: true,
                    ..Default::default()
                },
            },
        ];
        if code {
            runs.insert(
                1,
                InlineRun {
                    text: "src/main.rs".into(),
                    style: InlineStyle {
                        code: true,
                        ..Default::default()
                    },
                },
            );
        }
        for mut row in rows(&Block::Paragraph { runs }) {
            row.word_start = word_start;
            word_start += word_count(&row);
            result.push(Arc::new(row));
        }
    }
    result
}

struct DrawRoot(Option<AnyElement>);

impl Render for DrawRoot {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        self.0.take().unwrap()
    }
}

fn draw(
    visual: &mut VisualTestContext,
    prepared: &[Arc<PreparedRow>],
    fade: Option<&crate::reveal::WordFades>,
    now: Instant,
    width: f32,
) {
    visual.draw(
        point(px(0.), px(0.)),
        size(
            AvailableSpace::Definite(px(width)),
            AvailableSpace::MinContent,
        ),
        |_, cx| {
            let child = div()
                .w(px(width))
                .text_size(px(14.))
                .font_family(crate::fonts::UI)
                .line_height(px(24.))
                .children(prepared.iter().enumerate().map(|(index, prepared)| {
                    body_selected(
                        &prepared.row,
                        format!("fixture-{index}").into(),
                        Tone::Normal,
                        fade,
                        now,
                        None,
                        Some(prepared),
                    )
                }));
            cx.new(|_| DrawRoot(Some(child.into_any_element())))
                .into_any_element()
        },
    );
}

fn capture(
    visual: &mut VisualTestContext,
    prepared: &[Arc<PreparedRow>],
    fade: Option<&crate::reveal::WordFades>,
    now: Instant,
    width: f32,
) -> (usize, usize, Vec<gpui::Bounds<Pixels>>) {
    OBSERVED.with_borrow_mut(|observed| *observed = Some(Vec::new()));
    draw(visual, prepared, fade, now, width);
    let observed = OBSERVED.with_borrow_mut(|observed| observed.take().unwrap());
    (
        observed.len(),
        observed.iter().map(|(_, highlights)| highlights).sum(),
        observed.iter().map(|(layout, _)| layout.bounds()).collect(),
    )
}

#[gpui::test]
fn settled_render_counts_and_geometry(cx: &mut TestAppContext) {
    #[cfg(target_os = "macos")]
    let cx = &mut TestAppContext::build_with_text_system(
        cx.dispatcher.clone(),
        Some("settled_text_render"),
        Arc::new(gpui_apple::AppleTextSystem::new()),
    );
    let visual = cx.add_empty_window();
    let now = Instant::now();
    for code in [false, true] {
        let source = fixture(code);
        let mut cache = RenderCache::default();
        cache.begin_frame(&None, &None);
        let prepared: Vec<_> = source
            .iter()
            .enumerate()
            .map(|(index, row)| cache.get((index, 0), row))
            .collect();
        let words = source.iter().map(|row| word_count(row)).sum();
        for width in [360., 640.] {
            let mut paced = PacedText::new("", true, now);
            paced.words(words, now);
            paced.fades.advance(now + Duration::from_millis(160));
            let active = capture(visual, &prepared, Some(&paced.fades), now, width);
            paced.fades.advance(now + Duration::from_secs(1));
            let settled = capture(visual, &prepared, Some(&paced.fades), now, width);
            let reopened = capture(visual, &prepared, None, now, width);
            assert_eq!(active.2, settled.2);
            assert_eq!(active.0, settled.0);
            assert!(active.1 > settled.1);
            assert_eq!(settled, reopened);
            if code {
                assert!(settled.0 <= 24, "{} text elements", settled.0);
                assert!(settled.1 <= 36, "{} style ranges", settled.1);
            } else {
                assert_eq!(settled.0, 6);
                assert_eq!(settled.1, 24);
            }
            paced.words(words + 1, now + Duration::from_secs(1));
            assert_eq!(paced.fades.opacity(0), 1.);
            assert!(paced.fades.intersects(words..words + 1));
            paced.settle(now + Duration::from_secs(1));
        }
    }
}

#[test]
fn interrupted_reveal_and_late_append_preserve_word_fades() {
    let now = Instant::now();
    let mut paced = PacedText::new("one partial", true, now);
    assert!(paced.step(now + Duration::from_millis(30), true));
    assert_eq!(paced.text(), "one");
    paced.words(1, now + Duration::from_millis(30));
    assert!(paced.step(now + Duration::from_millis(60), false));
    assert_eq!(paced.text(), "one partial");
    paced.words(2, now + Duration::from_millis(60));
    paced.fades.advance(now + Duration::from_secs(1));
    assert!(!paced.active(now + Duration::from_secs(1)));
    let appended = now + Duration::from_millis(1100);
    paced.update("one partial more café", appended);
    assert!(paced.behind());
    assert!(paced.step(appended + Duration::from_millis(30), true));
    assert_eq!(paced.text(), "one partial more ");
    paced.words(3, appended + Duration::from_millis(30));
    assert_eq!(paced.fades.opacity(0), 1.);
    assert_eq!(paced.fades.opacity(1), 1.);
    assert!(paced.step(appended + Duration::from_millis(180), true));
    assert_eq!(paced.text(), "one partial more café");
    paced.words(4, appended + Duration::from_millis(180));
    paced.settle(appended + Duration::from_millis(180));
    assert!(!paced.active(appended + Duration::from_millis(180)));
    assert!(!paced.fades.intersects(0..4));
    let reopened = PacedText::new(paced.text(), false, appended);
    assert_eq!(reopened.text(), paced.text());
    assert!(!reopened.active(appended));
}

#[gpui::test]
fn mixed_code_flow_preserves_bytes_and_bounds(cx: &mut TestAppContext) {
    #[cfg(target_os = "macos")]
    let cx = &mut TestAppContext::build_with_text_system(
        cx.dispatcher.clone(),
        Some("mixed_code_flow"),
        Arc::new(gpui_apple::AppleTextSystem::new()),
    );
    let visual = cx.add_empty_window();
    let now = Instant::now();
    for scale in [1., 1.25, 1.3, 1.4, 2.6] {
        visual.simulate_scale_factor_change(scale);
        let long_path = format!("{}main.rs", "long_directory/".repeat(14));
        for path in ["x", "npm test", "src/main.rs", long_path.as_str()] {
            let mut runs = vec![
                InlineRun {
                    text: "café  e\u{301} 👩‍💻 中文\n\nBefore ".into(),
                    style: Default::default(),
                },
                InlineRun {
                    text: "bold ".into(),
                    style: InlineStyle {
                        bold: true,
                        ..Default::default()
                    },
                },
                InlineRun {
                    text: "linked words ".into(),
                    style: InlineStyle {
                        italic: true,
                        link: Some("https://example.com".into()),
                        ..Default::default()
                    },
                },
                InlineRun {
                    text: path.into(),
                    style: InlineStyle {
                        code: true,
                        ..Default::default()
                    },
                },
                InlineRun {
                    text: "  after\nnew line and more 中文文字".repeat(3),
                    style: Default::default(),
                },
                InlineRun {
                    text: "last.rs".into(),
                    style: InlineStyle {
                        code: true,
                        ..Default::default()
                    },
                },
            ];
            let mut cache = RenderCache::default();
            let mut previous_words = 0;
            for append in [false, true] {
                if append {
                    runs.push(InlineRun {
                        text: " appended café 👩‍💻".into(),
                        style: InlineStyle {
                            italic: true,
                            ..Default::default()
                        },
                    });
                }
                let source: Vec<_> = rows(&Block::Paragraph { runs: runs.clone() })
                    .into_iter()
                    .map(Arc::new)
                    .collect();
                cache.begin_frame(&None, &Some((0, "café".into())));
                let prepared: Vec<_> = source
                    .iter()
                    .enumerate()
                    .map(|(index, row)| cache.get((index, 0), row))
                    .collect();
                let words = source.iter().map(|row| word_count(row)).sum();
                for width in [80., 160., 360., 640.] {
                    let mut paced = PacedText::new("", true, now);
                    if append {
                        paced.words(previous_words, now);
                        paced.fades.advance(now + Duration::from_secs(1));
                        paced.words(words, now + Duration::from_secs(1));
                        paced.fades.advance(now + Duration::from_millis(1160));
                        assert_eq!(paced.fades.opacity(0), 1.);
                        assert!(paced.fades.opacity(words - 1) < 1.);
                    } else {
                        paced.words(words, now);
                        paced.fades.advance(now + Duration::from_millis(160));
                    }
                    let active = capture(visual, &prepared, Some(&paced.fades), now, width);
                    paced.fades.advance(now + Duration::from_secs(2));
                    let settled = capture(visual, &prepared, Some(&paced.fades), now, width);
                    let reopened = capture(visual, &prepared, None, now, width);
                    assert_eq!(active.2, settled.2);
                    assert_eq!(settled, reopened);
                    for bounds in &settled.2 {
                        assert!(
                            bounds.left() >= px(0.) && bounds.right() <= px(width + 1.),
                            "{bounds:?} at {width}"
                        );
                    }
                    for (index, first) in settled.2.iter().enumerate() {
                        for second in &settled.2[index + 1..] {
                            let overlap_width =
                                first.right().min(second.right()) - first.left().max(second.left());
                            let overlap_height =
                                first.bottom().min(second.bottom()) - first.top().max(second.top());
                            assert!(
                                overlap_width <= px(0.1) || overlap_height <= px(0.1),
                                "overlapping text {first:?} and {second:?}"
                            );
                        }
                    }
                    begin_capture();
                    draw(visual, &prepared, None, now, width);
                    let bytes: usize = finish_capture()
                        .iter()
                        .flat_map(TextLayout::line_layouts)
                        .map(|line| line.unwrapped_layout.len)
                        .sum();
                    let expected: usize = prepared
                        .iter()
                        .map(|row| row.selection.len() - row.selection.matches('\n').count())
                        .sum();
                    assert_eq!(bytes, expected, "rendered text must retain source bytes");
                }
                previous_words = words;
            }
        }
    }
}

#[gpui::test]
fn code_flow_keeps_links_menus_and_hover(cx: &mut TestAppContext) {
    use std::rc::Rc;
    struct Links;
    impl Render for Links {
        fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
            let runs = vec![
                InlineRun {
                    text: "Before ".into(),
                    style: Default::default(),
                },
                InlineRun {
                    text: "linked words ".into(),
                    style: InlineStyle {
                        link: Some("https://example.com".into()),
                        ..Default::default()
                    },
                },
                InlineRun {
                    text: "main.rs".into(),
                    style: InlineStyle {
                        code: true,
                        ..Default::default()
                    },
                },
                InlineRun {
                    text: " after".into(),
                    style: Default::default(),
                },
            ];
            div().w(px(360.)).child(rich_inline(
                &runs,
                "links".into(),
                Tone::Normal,
                None,
                Instant::now(),
                0,
                14.,
                None,
            ))
        }
    }
    let menus = Rc::new(RefCell::new(Vec::new()));
    let paths = Rc::new(RefCell::new(Vec::new()));
    cx.update(|cx| {
        let menus = menus.clone();
        cx.on_action(move |action: &LinkMenu, _| menus.borrow_mut().push(action.url.clone()));
        let paths = paths.clone();
        cx.on_action(move |action: &crate::file_editor::OpenPath, _| {
            paths.borrow_mut().push(action.0.clone())
        });
    });
    begin_capture();
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| cx.new(|_| Links))
            .unwrap()
    });
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    let layouts = finish_capture();
    let link = layouts[0].position_for_index(7).unwrap() + point(px(1.), px(12.));
    let chip = layouts[1].bounds().center();
    visual.simulate_click(link, Default::default());
    assert_eq!(visual.opened_url().as_deref(), Some("https://example.com"));
    visual.simulate_event(gpui::MouseDownEvent {
        position: link,
        button: MouseButton::Right,
        click_count: 1,
        modifiers: Default::default(),
        first_mouse: false,
    });
    assert_eq!(
        menus.borrow().last().map(String::as_str),
        Some("https://example.com")
    );
    visual.simulate_click(chip, Default::default());
    assert_eq!(paths.borrow().last().map(String::as_str), Some("main.rs"));
    visual.simulate_event(gpui::MouseDownEvent {
        position: chip,
        button: MouseButton::Right,
        click_count: 1,
        modifiers: Default::default(),
        first_mouse: false,
    });
    assert_eq!(menus.borrow().last().map(String::as_str), Some("main.rs"));
    visual.simulate_mouse_move(chip, None, Default::default());
    visual.refresh().unwrap();
    window
        .update(visual, |_, window, _| {
            assert!(
                window
                    .painted_quads()
                    .iter()
                    .any(|quad| quad.background == content(0.13).into())
            );
        })
        .unwrap();
}

#[gpui::test]
fn multiword_code_keeps_one_line_when_it_fits(cx: &mut TestAppContext) {
    #[cfg(target_os = "macos")]
    let cx = &mut TestAppContext::build_with_text_system(
        cx.dispatcher.clone(),
        Some("multiword_code"),
        Arc::new(gpui_apple::AppleTextSystem::new()),
    );
    let visual = cx.add_empty_window();
    let now = Instant::now();
    for scale in [1., 1.25, 1.3, 1.4, 1.5, 2., 2.5, 2.6, 3.] {
        visual.simulate_scale_factor_change(scale);
        let line_height = visual.update(|window, _| window.pixel_snap(px(24.)));
        for text in [
            "npm test",
            "git status --short",
            "a b",
            "cargo test --locked",
        ] {
            let source = Arc::new(
                rows(&Block::Paragraph {
                    runs: vec![
                        InlineRun {
                            text: "Use ".into(),
                            style: Default::default(),
                        },
                        InlineRun {
                            text: text.into(),
                            style: InlineStyle {
                                code: true,
                                ..Default::default()
                            },
                        },
                        InlineRun {
                            text: " to run the tests.".into(),
                            style: Default::default(),
                        },
                    ],
                })
                .remove(0),
            );
            let mut cache = RenderCache::default();
            cache.begin_frame(&None, &None);
            let prepared = vec![cache.get((0, 0), &source)];
            let captured = capture(visual, &prepared, None, now, 640.);
            assert_eq!(captured.0, 3);
            assert!(
                captured
                    .2
                    .iter()
                    .all(|bounds| (bounds.size.height - line_height).abs() < px(0.01)),
                "{text}: {:?}",
                captured.2
            );
        }
    }
}
