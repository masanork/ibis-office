// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Reading a slide's animations (`p:timing`): the effects of its main
//! sequence (`nodeType="mainSeq"`), each with its shape, class, start, look
//! and times. The structure PowerPoint writes is a sequence of click groups
//! (`p:par`), each of timed groups (`p:par` with a `delay`), each of
//! effects (`p:par` whose `p:cTn` has a `presetClass`); see ECMA-376
//! Part 1, 19.5 (Animation). Interactive sequences (triggers) are not read.

use std::collections::HashMap;

use ibis_doc_model::{Animation, AnimationAfterEffect, AnimationClass, AnimationLook, AnimationRepeat, AnimationSide, AnimationStart, AnimationTrigger, ShapeId};
use roxmltree::Node;

use crate::xml::{child, elements};

fn ctn<'a, 'i>(node: Node<'a, 'i>) -> Option<Node<'a, 'i>> {
    child(node, "cTn")
}

fn is_true(v: Option<&str>) -> bool {
    matches!(v, Some("1" | "true"))
}

/// A time node's own start delay (`stCondLst/cond/@delay`), milliseconds;
/// 0 when `indefinite` or absent.
fn delay(ctn: Node) -> u32 {
    child(ctn, "stCondLst")
        .into_iter()
        .flat_map(elements)
        .filter_map(|c| c.attribute("delay"))
        .filter_map(|d| d.parse::<u32>().ok())
        .max()
        .unwrap_or(0)
}

/// Parse the trigger condition from an interactive sequence's cTn.
fn parse_interactive_trigger(interactive_ctn: Node, shapes: &HashMap<String, ShapeId>, groups: &HashMap<String, Vec<ShapeId>>) -> AnimationTrigger {
    let Some(st_cond_lst) = child(interactive_ctn, "stCondLst") else { return AnimationTrigger::None };
    for cond in elements(st_cond_lst) {
        if cond.tag_name().name() != "cond" {
            continue;
        }
        // A. オブジェクトのクリック時: evt="onClick" かつ spTgt
        if cond.attribute("evt") == Some("onClick") {
            if let Some(spid) = cond.descendants().find(|n| n.is_element() && n.tag_name().name() == "spTgt").and_then(|n| n.attribute("spid")) {
                if let Some(&shape) = shapes.get(spid) {
                    return AnimationTrigger::OnShapeClick(shape);
                }
                if let Some(members) = groups.get(spid).filter(|m| !m.is_empty()) {
                    return AnimationTrigger::OnGroupClick { native_id: spid.to_string(), shapes: members.clone() };
                }
            }
        }
        // B. メディアブックマーク時: p14:mediaBmKTrigger
        if let Some(bmk_node) = cond.descendants().find(|n| n.is_element() && n.tag_name().name() == "mediaBmKTrigger") {
            let bmk_name = bmk_node.attribute("bmkName").unwrap_or_default();
            if let Some(spid) = bmk_node.descendants().find(|n| n.is_element() && n.tag_name().name() == "spTgt").and_then(|n| n.attribute("spid")) {
                if let Some(&shape) = shapes.get(spid) {
                    return AnimationTrigger::OnMediaBookmark { shape, bookmark: bmk_name.to_string() };
                }
            }
        }
    }
    AnimationTrigger::None
}

/// The effects of `timing` (a slide's `p:timing`), in order, for the shapes
/// `shapes` names by drawing id.
pub(crate) fn read(
    slide: Node,
    shapes: &HashMap<String, ShapeId>,
    groups: &HashMap<String, Vec<ShapeId>>,
    group_pivots: &HashMap<String, (i32, i32)>,
    colors: Option<&crate::theme::Colors<'_>>,
) -> Vec<Animation> {
    let Some(timing) = child(slide, "timing") else { return Vec::new() };
    let mut chart_builds: HashMap<String, (ibis_doc_model::ChartBuildStep, bool)> = HashMap::new();
    let mut diagram_builds: HashMap<String, ibis_doc_model::DiagramBuildStep> = HashMap::new();
    if let Some(bld_lst) = child(timing, "bldLst") {
        for bld_chart in elements(bld_lst).filter(|n| n.tag_name().name() == "bldChart") {
            if let Some(spid) = bld_chart.attribute("spid") {
                let step = bld_chart
                    .attribute("build")
                    .or_else(|| bld_chart.attribute("bld"))
                    .and_then(ibis_doc_model::ChartBuildStep::parse)
                    .unwrap_or_default();
                let anim_bg = bld_chart.attribute("animBg").map(|v| v == "1" || v == "true").unwrap_or(true);
                chart_builds.insert(spid.to_string(), (step, anim_bg));
            }
        }
        for bld_graphic in elements(bld_lst).filter(|n| n.tag_name().name() == "bldGraphic") {
            if let Some(spid) = bld_graphic.attribute("spid") {
                if let Some(bld_chart) = bld_graphic.descendants().find(|n| n.is_element() && n.tag_name().name() == "bldChart") {
                    let step = bld_chart
                        .attribute("bld")
                        .or_else(|| bld_chart.attribute("build"))
                        .and_then(ibis_doc_model::ChartBuildStep::parse)
                        .unwrap_or_default();
                    let anim_bg = bld_chart.attribute("animBg").map(|v| v == "1" || v == "true").unwrap_or(true);
                    chart_builds.insert(spid.to_string(), (step, anim_bg));
                }
            }
        }
        for bld_dgm in elements(bld_lst).filter(|n| n.tag_name().name() == "bldDgm") {
            if let Some(spid) = bld_dgm.attribute("spid") {
                let step = bld_dgm.attribute("bld").and_then(ibis_doc_model::DiagramBuildStep::parse).unwrap_or_default();
                diagram_builds.insert(spid.to_string(), step);
            }
        }
    }
    let targets = Targets { shapes, groups, group_pivots, chart_builds: &chart_builds, diagram_builds: &diagram_builds, colors };
    let mut out = Vec::new();

    // 1. Main sequence effects (mainSeq)
    if let Some(main) = timing.descendants().find(|n| n.is_element() && n.tag_name().name() == "cTn" && n.attribute("nodeType") == Some("mainSeq")) {
        read_time_node(main,0,&targets,&AnimationTrigger::None,&mut out);
    }

    // 2. Interactive sequence effects (interactiveSeq, triggers)
    for interactive in timing.descendants().filter(|n| n.is_element() && n.tag_name().name() == "cTn" && n.attribute("nodeType") == Some("interactiveSeq")) {
        let trigger = parse_interactive_trigger(interactive, shapes, groups);
        if trigger.is_none() {
            continue;
        }
        read_time_node(interactive,0,&targets,&trigger,&mut out);
    }

    out
}

/// CT_TimeNodeList permits nested parallel and sequential containers. Office
/// versions need not insert exactly the same number of wrapper nodes.
fn read_time_node(node:Node,parent_delay:u32,targets:&Targets<'_>,trigger:&AnimationTrigger,out:&mut Vec<Animation>){
    if node.attribute("presetClass").is_some(){read_effects(node,parent_delay,targets,trigger,out);return;}
    let nested_delay=parent_delay.saturating_add(delay(node));
    for timed in child(node,"childTnLst").into_iter().flat_map(elements){
        if let Some(next)=ctn(timed){read_time_node(next,nested_delay,targets,trigger,out);}
    }
}

/// The effects of time node `e`: one for its shape, or (a group, 19.5: the
/// group is the target) one for each shape of the group, the first starting
/// as `e` does and the others with it.
fn read_effects(e: Node, group_delay: u32, targets: &Targets, trigger: &AnimationTrigger, out: &mut Vec<Animation>) {
    let Some(target_node) = e.descendants().find(|n| n.is_element() && n.tag_name().name() == "spTgt") else { return };
    let Some(target) = target_node.attribute("spid") else { return };
    // A range of paragraphs (`txEl/pRg`).
    let paragraph = child(target_node, "txEl")
        .and_then(|t| child(t, "pRg"))
        .and_then(|r| Some((r.attribute("st")?.parse::<u32>().ok()?, r.attribute("end")?.parse::<u32>().ok()?)))
        .filter(|(st, end)| st <= end);
    // A range of characters (`txEl/charRg`).
    let chars = child(target_node, "txEl")
        .and_then(|t| child(t, "charRg"))
        .and_then(|r| Some((r.attribute("st")?.parse::<u32>().ok()?, r.attribute("end")?.parse::<u32>().ok()?)))
        .filter(|(st, end)| st <= end);
    // A chart element (`cEl/cChartEl`).
    let chart_element = child(target_node, "cEl")
        .and_then(|c| child(c, "cChartEl"))
        .and_then(|el| {
            let s = el.attribute("seriesIdx")?.parse::<i32>().ok()?;
            let c = el.attribute("categoryIdx")?.parse::<i32>().ok()?;
            Some(ibis_doc_model::ChartAnimationElement { series_idx: s, category_idx: c })
        });
    let chart_build = targets.chart_builds.get(target).copied();
    // A diagram element (`graphicEl/dgm` or `subSp`).
    let dgm_element = child(target_node, "graphicEl")
        .and_then(|g| child(g, "dgm"))
        .and_then(|d| {
            let raw_id = d.attribute("id")?;
            let id = raw_id
                .strip_prefix('{')
                .and_then(|s| s.strip_suffix("-0000-0000-0000-000000000000}"))
                .and_then(|hex| u64::from_str_radix(hex, 16).ok())
                .map(|n| n.to_string())
                .unwrap_or_else(|| raw_id.to_string());
            let bld_step = d.attribute("bldStep").unwrap_or("sp").to_string();
            Some(ibis_doc_model::DiagramAnimationElement { id, bld_step })
        })
        .or_else(|| {
            child(target_node, "subSp").and_then(|s| {
                let id = s.attribute("spid")?.to_string();
                Some(ibis_doc_model::DiagramAnimationElement { id, bld_step: "sp".to_string() })
            })
        });
    let dgm_build = targets.diagram_builds.get(target).copied();
    if let Some(&shape) = targets.shapes.get(target) {
        out.extend(read_effect_with_colors(e, group_delay, shape, trigger.clone(), targets.colors).map(|a| Animation {
            paragraph,
            chars: if paragraph.is_some() { None } else { chars },
            chart_element,
            chart_build,
            dgm_element,
            dgm_build,
            ..a
        }));
    } else if let Some(members) = targets.groups.get(target) {
        let pivot = targets.group_pivots.get(target).copied();
        for (i, &shape) in members.iter().enumerate() {
            if let Some(mut animation) = read_effect_with_colors(e, group_delay, shape, trigger.clone(), targets.colors) {
                if i > 0 {
                    animation.start = AnimationStart::WithPrevious;
                }
                animation.group_pivot = pivot;
                animation.chart_element = chart_element;
                animation.chart_build = chart_build;
                animation.dgm_element = dgm_element.clone();
                animation.dgm_build = dgm_build;
                out.push(animation);
            }
        }
    }
}

/// What the effects' targets (`p:spTgt/@spid`) are: drawing ids of shapes, and of groups (their shapes).
pub(crate) struct Targets<'a> {
    pub(crate) shapes: &'a HashMap<String, ShapeId>,
    pub(crate) groups: &'a HashMap<String, Vec<ShapeId>>,
    pub(crate) group_pivots: &'a HashMap<String, (i32, i32)>,
    pub(crate) chart_builds: &'a HashMap<String, (ibis_doc_model::ChartBuildStep, bool)>,
    pub(crate) diagram_builds: &'a HashMap<String, ibis_doc_model::DiagramBuildStep>,
    pub(crate) colors: Option<&'a crate::theme::Colors<'a>>,
}

pub(crate) fn read_effect(e: Node, group_delay: u32, shape: ShapeId, trigger: AnimationTrigger) -> Option<Animation> {
    read_effect_with_colors(e, group_delay, shape, trigger, None)
}

fn scale_by_speed(ms: u32, spd_pct: Option<u32>) -> u32 {
    match spd_pct {
        Some(s) if s > 0 && s != 100_000 => ((u64::from(ms) * 100_000) / u64::from(s)).min(u64::from(u32::MAX)) as u32,
        _ => ms,
    }
}

fn read_effect_with_colors(e: Node, group_delay: u32, shape: ShapeId, trigger: AnimationTrigger, colors: Option<&crate::theme::Colors<'_>>) -> Option<Animation> {
    let class = match e.attribute("presetClass")? {
        "entr" => AnimationClass::Entrance,
        "emph" => AnimationClass::Emphasis,
        "exit" => AnimationClass::Exit,
        "path" => AnimationClass::Path,
        "mediacall" => AnimationClass::Media,
        _ => return None,
    };
    let start = match e.attribute("nodeType") {
        Some("withEffect") => AnimationStart::WithPrevious,
        Some("afterEffect") => AnimationStart::AfterPrevious,
        _ => AnimationStart::OnClick,
    };
    let preset = (
        e.attribute("presetID").and_then(|v| v.parse().ok()).unwrap_or(0),
        e.attribute("presetSubtype").and_then(|v| v.parse().ok()).unwrap_or(0),
    );
    // The effect lasts as long as its longest behaviour (and its delay).
    let behaviours: Vec<Node> = child(e, "childTnLst").into_iter().flat_map(elements).collect();
    let effect_spd = e.attribute("spd").and_then(|v| v.parse::<i32>().ok()).filter(|&v| v != 0).map(i32::unsigned_abs);
    // (A behaviour that runs forward and back takes twice its `dur`. A preset played as its behaviours keeps that in the behaviours;
    // for the other looks the effect's own `auto_reverse` carries it, and `duration` is the one pass.)
    let longest = |doubled: bool| {
        let raw = behaviours
            .iter()
            .filter_map(|b| child(*b, "cBhvr").and_then(ctn))
            .map(|c| {
                let bhvr_spd = c.attribute("spd").and_then(|v| v.parse::<i32>().ok()).filter(|&v| v != 0).map(i32::unsigned_abs);
                let dur = scale_by_speed(c.attribute("dur").and_then(|d| d.parse::<u32>().ok()).unwrap_or(0), bhvr_spd);
                delay(c) + dur * if doubled && is_true(c.attribute("autoRev")) { 2 } else { 1 }
            })
            .max()
            .unwrap_or(0);
        scale_by_speed(raw, effect_spd)
    };
    let has = |name: &str| behaviours.iter().any(|b| b.tag_name().name() == name);
    let side = |subtype: u32| match subtype {
        1 => Some(AnimationSide::Top),
        2 => Some(AnimationSide::Right),
        4 => Some(AnimationSide::Bottom),
        8 => Some(AnimationSide::Left),
        _ => None,
    };
    let mut behaviors = parse_behaviors(&behaviours, e.attribute("fill") == Some("hold"));
    if let Some(spd) = effect_spd {
        for bh in &mut behaviors {
            bh.begin = scale_by_speed(bh.begin, Some(spd));
            bh.dur = scale_by_speed(bh.dur, Some(spd));
        }
    }
    let motion_path = behaviours.iter().find(|b| b.tag_name().name() == "animMotion").and_then(|b| {
        if let Some(pts) = b.attribute("path").and_then(path_points) {
            return Some(pts);
        }
        let pair = |name: &str| {
            let n = child(*b, name)?;
            Some((n.attribute("x")?.parse::<i32>().ok()?, n.attribute("y")?.parse::<i32>().ok()?))
        };
        if let Some((bx, by)) = pair("by") {
            return Some(vec![(bx, by)]);
        }
        if let Some((tx, ty)) = pair("to") {
            let (fx, fy) = pair("from").unwrap_or((0, 0));
            return Some(vec![(tx - fx, ty - fy)]);
        }
        None
    });
    // 再生・一時停止・停止: the call (`p:cmd`) says which; the preset (1, 2, 3) when it names none we know.
    let media_command = behaviours.iter().find(|b| b.tag_name().name() == "cmd").and_then(|b| b.attribute("cmd")).and_then(|c| {
        if c.starts_with("play") || c == "resume" {
            Some(ibis_doc_model::MediaCommand::Play)
        } else if c == "togglePause" || c == "pause" {
            Some(ibis_doc_model::MediaCommand::Pause)
        } else if c == "stop" {
            Some(ibis_doc_model::MediaCommand::Stop)
        } else {
            None
        }
    });
    let look = match class {
        AnimationClass::Media => AnimationLook::Media(media_command.or(match preset.0 {
            1 => Some(ibis_doc_model::MediaCommand::Play),
            2 => Some(ibis_doc_model::MediaCommand::Pause),
            3 => Some(ibis_doc_model::MediaCommand::Stop),
            _ => None,
        })?),
        AnimationClass::Entrance | AnimationClass::Exit => match preset.0 {
            1 => AnimationLook::Appear,
            // Fly in / out (スライドイン): the subtype says from where.
            2 => match side(preset.1) {
                Some(s) => AnimationLook::Fly(s),
                None if !behaviors.is_empty() => AnimationLook::Preset,
                None => AnimationLook::Fade,
            },
            10 => AnimationLook::Fade,
            22 => match side(preset.1) {
                Some(s) => AnimationLook::Wipe(s),
                None if !behaviors.is_empty() => AnimationLook::Preset,
                None => AnimationLook::Wipe(AnimationSide::Bottom),
            },
            // PowerPoint MsoAnimEffectZoom=23 and Float=30 (Microsoft's enum). PowerPoint's own ズーム entry
            // is 53 (subtype 16 from the object's centre, 528 from the slide's centre; issue #62).
            // ズーム (grow from the shape's centre, subtype 16 in / 32 out); its other options move the shape too.
            23 if has("animScale") || matches!((class, preset.1), (AnimationClass::Entrance, 16) | (AnimationClass::Exit, 32)) => AnimationLook::Zoom,
            // フロート as ibisOffice wrote it (a fade with a short rise); PowerPoint's own (with `ppt_y` and a turn) is played as it says.
            30 if !has("anim") => AnimationLook::Float(if preset.1 == 2 { AnimationSide::Bottom } else { AnimationSide::Top }),
            // クレジット (28) as PowerPoint writes it has the behaviours that move the shape; ibisOffice's own had none.
            28 if !has("anim") => AnimationLook::Replay,
            // Every other preset (including 53, PowerPoint's own ズーム, and スライドイン from a corner) is played as
            // the behaviours the file gives it say, not as a fade.
            _ if !behaviors.is_empty() => AnimationLook::Preset,
            _ if has("animEffect") || has("anim") || has("animScale") => AnimationLook::Fade,
            _ => AnimationLook::Appear,
        },
        AnimationClass::Emphasis => {
            // スピン and 拡大／収縮 as one behaviour alone; every other emphasis effect (シマー, ウェーブ…) is its behaviours.
            let only = |name: &str| behaviours.len() == 1 && behaviours[0].tag_name().name() == name;
            let turns_3d = behaviours.first().is_some_and(|b| b.descendants().filter(|n| n.tag_name().name() == "attrName").any(|n| n.text().is_some_and(|t| t.trim().starts_with("3d."))));
            if only("animRot") && !turns_3d && behaviours[0].attribute("by").is_some() {
                AnimationLook::Spin(behaviours[0].attribute("by").and_then(|v| v.parse().ok()).unwrap_or(0))
            } else if let Some(by) = behaviours.iter().find(|b| only(b.tag_name().name()) && b.tag_name().name() == "animScale").and_then(|b| child(*b, "by")) {
                let v = |a: &str| by.attribute(a).and_then(|v| v.parse().ok()).unwrap_or(100_000);
                AnimationLook::Scale(v("x"), v("y"))
            } else if !behaviors.is_empty() {
                AnimationLook::Preset
            } else {
                AnimationLook::Other
            }
        }
        AnimationClass::Path => match motion_path {
            Some(ref points) => points.last().map_or(AnimationLook::Other, |&(x, y)| AnimationLook::Motion(x, y)),
            // A path the model cannot draw as lines: kept as written.
            None if !behaviors.is_empty() => AnimationLook::Preset,
            None => AnimationLook::Other,
        },
    };
    // A path of more than one straight line (カスタム軌跡) keeps its points.
    let path = match (&class, motion_path) {
        (AnimationClass::Path, Some(points)) if !is_single_line(&points) => points,
        _ => Vec::new(),
    };
    // The timed group's delay is where 直前の動作の後 puts it (the end of
    // what came before, which the slide show works out itself): only the
    // effect's own delay is its delay.
    let _ = group_delay;
    // A preset played as its behaviours say keeps their time options (smooth start, auto-reverse…) on the behaviours.
    let preset_look = look == AnimationLook::Preset;
    let duration = longest(preset_look);
    // `repeatCount="indefinite"` stops at the next click (`endCondLst`, the default here) or at the end of the slide (`endSync`).
    let until_end_of_slide = child(e, "endSync").is_some();
    let repeat_from_thousandths = |n: u32| -> AnimationRepeat {
        if n == 0 || n == 1000 {
            AnimationRepeat::None
        } else if n % 1000 == 0 {
            AnimationRepeat::Count(n / 1000)
        } else {
            // Fractional counts (`500` = 0.5 times, `2500` = 2.5 times): stops partway through the pass.
            AnimationRepeat::Thousandths(n)
        }
    };
    let parse_repeat = |val: &str| -> AnimationRepeat {
        if val == "indefinite" {
            if until_end_of_slide { AnimationRepeat::UntilEndOfSlide } else { AnimationRepeat::UntilNextClick }
        } else if let Ok(n) = val.parse::<u32>() {
            repeat_from_thousandths(n)
        } else {
            AnimationRepeat::None
        }
    };
    let auto_reverse = e.attribute("autoRev")
        .or_else(|| behaviours.iter().filter(|_| !preset_look).filter_map(|b| child(*b, "cBhvr").and_then(ctn)).find_map(|c| c.attribute("autoRev")))
        .is_some_and(|v| v == "1" || v == "true");
    let raw_repeat_count = e.attribute("repeatCount")
        .or_else(|| behaviours.iter().filter(|_| !preset_look).filter_map(|b| child(*b, "cBhvr").and_then(ctn)).find_map(|c| c.attribute("repeatCount")));
    let mut repeat_count = raw_repeat_count.map_or(AnimationRepeat::None, parse_repeat);
    let raw_repeat_dur = e.attribute("repeatDur")
        .or_else(|| behaviours.iter().filter(|_| !preset_look).filter_map(|b| child(*b, "cBhvr").and_then(ctn)).find_map(|c| c.attribute("repeatDur")));
    if let Some(rd) = raw_repeat_dur {
        if rd == "indefinite" {
            if raw_repeat_count.is_none() {
                repeat_count = if until_end_of_slide { AnimationRepeat::UntilEndOfSlide } else { AnimationRepeat::UntilNextClick };
            }
        } else if let Ok(rd_ms) = rd.parse::<u32>() {
            let pass_ms = duration.saturating_mul(if auto_reverse { 2 } else { 1 });
            if pass_ms > 0 {
                let rd_thousandths = ((u64::from(rd_ms) * 1000 + u64::from(pass_ms) / 2) / u64::from(pass_ms)).min(u64::from(u32::MAX)) as u32;
                let from_dur = repeat_from_thousandths(rd_thousandths);
                repeat_count = match (raw_repeat_count, repeat_count) {
                    (None, _) => from_dur,
                    (Some(_), AnimationRepeat::UntilNextClick | AnimationRepeat::UntilEndOfSlide) => from_dur,
                    (Some(_), AnimationRepeat::Count(c)) if u64::from(c) * 1000 > u64::from(rd_thousandths) => from_dur,
                    (Some(_), AnimationRepeat::Thousandths(t)) if t > rd_thousandths => from_dur,
                    (Some(_), existing) => existing,
                };
            }
        }
    }
    let accel = e.attribute("accel")
        .or_else(|| behaviours.iter().filter(|_| !preset_look).filter_map(|b| child(*b, "cBhvr").and_then(ctn)).find_map(|c| c.attribute("accel")))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let decel = e.attribute("decel")
        .or_else(|| behaviours.iter().filter(|_| !preset_look).filter_map(|b| child(*b, "cBhvr").and_then(ctn)).find_map(|c| c.attribute("decel")))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    // 巻き戻し is `fill="remove"` on the effect (what PowerPoint writes; measured on its files, 2026-11-15); `rewind="1"` is how ibisOffice wrote it.
    let rewind = e.attribute("rewind")
        .or_else(|| behaviours.iter().filter(|_| !preset_look).filter_map(|b| child(*b, "cBhvr").and_then(ctn)).find_map(|c| c.attribute("rewind")))
        .is_some_and(|v| v == "1" || v == "true")
        || e.attribute("fill") == Some("remove");
    // The effect's behaviours that run at a click (`p:subTnLst`): 淡色表示 is a colour (`ppt_c`) taken at the next click, 非表示 a `style.visibility`
    // of hidden at the end of the effect (`masterRel="sameClick"`) or at the next click.
    let after_effect = child(e, "subTnLst")
        .and_then(|list| {
            list.children().filter(|n| n.is_element()).find_map(|b| {
                let master = child(b, "cBhvr").and_then(ctn)?.attribute("masterRel")?;
                let to = child(b, "to")?;
                match b.tag_name().name() {
                    "set" if child(to, "strVal").and_then(|v| v.attribute("val")) == Some("hidden") => Some(if master == "sameClick" { AnimationAfterEffect::Hide } else { AnimationAfterEffect::HideOnNextClick }),
                    "animClr" if master == "nextClick" => {
                        let rgb = colors
                            .and_then(|c| c.in_parent(to, None))
                            .map(|c| c.rgb)
                            .or_else(|| child(to, "srgbClr").and_then(|c| c.attribute("val")).and_then(|v| u32::from_str_radix(v, 16).ok()).map(ibis_doc_model::Rgb))?;
                        Some(AnimationAfterEffect::Dim(rgb))
                    }
                    _ => None,
                }
            })
        })
        .unwrap_or(AnimationAfterEffect::Hold);
    // The text a word or a letter at a time (`p:iterate`).
    let text_unit = child(e, "iterate").and_then(|it| {
        let unit = match it.attribute("type") {
            Some("wd") => ibis_doc_model::TextUnit::Word,
            Some("lt") => ibis_doc_model::TextUnit::Letter,
            _ => return None,
        };
        let gap = if let Some(v) = child(it, "tmPct").and_then(|t| t.attribute("val")).and_then(|v| v.parse().ok()) {
            ibis_doc_model::UnitGap::Percent(v)
        } else {
            ibis_doc_model::UnitGap::Millis(child(it, "tmAbs").and_then(|t| t.attribute("val")).and_then(|v| v.parse().ok())?)
        };
        Some((unit, gap))
    });
    Some(Animation { shape, class, start, look, delay: delay(e), duration, preset, repeat_count, auto_reverse, rewind, accel, decel, after_effect, trigger, paragraph: None, chars: None, text_unit, path, chart_element: None, chart_build: None, dgm_element: None, dgm_build: None, behaviors: if look == AnimationLook::Preset { behaviors } else { Vec::new() }, group_pivot: None })
}

/// An effect's behaviours as the model keeps them (`p:set`, `p:anim`, `p:animEffect`, `p:animScale`, `p:animRot`);
/// others (motion paths, colours) are left out.
fn parse_behaviors(nodes: &[Node], inherit_hold: bool) -> Vec<ibis_doc_model::AnimBehavior> {
    use ibis_doc_model::{AnimBehavior, BehaviorKind};
    let value = |v: Node| -> Option<String> {
        let inner = elements(v).next()?;
        inner.attribute("val").map(str::to_string)
    };
    let pair = |n: Option<Node>| -> Option<(i32, i32)> {
        let n = n?;
        Some((n.attribute("x")?.parse().ok()?, n.attribute("y")?.parse().ok()?))
    };
    // A behaviour kept as its XML, its target and time node ids to be filled in on writing.
    // Uses roxmltree ranges so any namespace prefix (`p:`, `pp:`, unprefixed) and quote style (`"`, `'`) works.
    let opaque = |b: Node| -> Option<BehaviorKind> {
        let doc_text = b.document().input_text();
        let base = b.range().start;
        let tgt_range = b.descendants().find(|n| n.is_element() && n.tag_name().name() == "tgtEl")?.range();
        let mut edits: Vec<(std::ops::Range<usize>, &'static str)> = vec![(tgt_range.start - base..tgt_range.end - base, "\u{1}")];
        for ctn_node in b.descendants().filter(|n| n.is_element() && n.tag_name().name() == "cTn") {
            if let Some(id_attr) = ctn_node.attribute_node("id") {
                let vr = id_attr.range_value();
                if vr.start >= base && vr.end <= b.range().end && !(vr.start >= tgt_range.start && vr.end <= tgt_range.end) {
                    edits.push((vr.start - base..vr.end - base, "\u{2}"));
                }
            }
        }
        edits.sort_by_key(|(r, _)| r.start);
        let raw = &doc_text[b.range()];
        let mut out = String::with_capacity(raw.len());
        let mut pos = 0;
        for (range, rep) in edits {
            if range.start >= pos && range.end <= raw.len() {
                out.push_str(&raw[pos..range.start]);
                out.push_str(rep);
                pos = range.end;
            }
        }
        out.push_str(&raw[pos..]);
        Some(BehaviorKind::Opaque(out))
    };
    // When the last of the behaviours ends: a change of size that says nothing of its fill takes its effect's only if it lasts to there (one that
    // ends before is gone from then on: the Zipper exit's scale of 1.23 s in an effect of 2 s is back to the shape's size after it, #89 against
    // PowerPoint's video, 5 settings at 2 s and 4 s).
    let end_of = |b: &Node| child(*b, "cBhvr").and_then(|c| ctn(c)).map_or(0, |t| delay(t) + t.attribute("dur").and_then(|d| d.parse::<u32>().ok()).unwrap_or(0));
    let last_end = nodes.iter().map(end_of).max().unwrap_or(0);
    let mut out = Vec::new();
    for b in nodes {
        let Some(bhvr) = child(*b, "cBhvr") else { continue };
        let time = ctn(bhvr);
        // Only a change of size (`p:animScale`) goes back at its end; the others' values stay (the Zipper's `ppt_y` of 1.23 s in 2 s does).
        let lasts = end_of(b) >= last_end || b.tag_name().name() != "animScale";
        let begin = time.map_or(0, delay);
        let bhvr_spd = time.and_then(|c| c.attribute("spd")).and_then(|v| v.parse::<i32>().ok()).filter(|&v| v != 0).map(i32::unsigned_abs);
        let dur = scale_by_speed(time.and_then(|c| c.attribute("dur")).and_then(|d| d.parse::<u32>().ok()).unwrap_or(0), bhvr_spd);
        let time_node = time;
        let flag = |a: &str| time_node.is_some_and(|c| is_true(c.attribute(a)));
        let behavior_time = ibis_doc_model::animation_preset::BehaviorTime {
            accel: time_node.and_then(|c| c.attribute("accel")).and_then(|v| v.parse().ok()).unwrap_or(0),
            decel: time_node.and_then(|c| c.attribute("decel")).and_then(|v| v.parse().ok()).unwrap_or(0),
            auto_rev: flag("autoRev"),
            // `by` alone is the amount added to the value the shape has (シマー's `by="(#ppt_w*0.10)"`), as `additive="sum"` says.
            sum: bhvr.attribute("additive") == Some("sum") || (b.tag_name().name() == "anim" && b.attribute("by").is_some() && b.attribute("from").is_none() && b.attribute("to").is_none()),
            // Held to the end of the effect by `fill="hold"` (or by running as long as the effect does, `dur="indefinite"`); a behaviour
            // that says nothing of it takes its effect's (PowerPoint's exit 43 writes `fill="hold"` on the effect only).
            remove: !(time_node.is_some_and(|c| c.attribute("fill") == Some("hold") || c.attribute("dur") == Some("indefinite") || (c.attribute("fill").is_none() && inherit_hold && lasts))),
            filter: time_node.and_then(|c| c.attribute("tmFilter")).map(ibis_doc_model::animation_preset::BehaviorTime::parse_filter).unwrap_or_default(),
        };
        let attr = child(bhvr, "attrNameLst").and_then(|l| child(l, "attrName")).and_then(|n| n.text()).map(|t| t.trim().to_string());
        let kind = match b.tag_name().name() {
            // A colour (`<p:to><p:clrVal>`) is kept as it is written (see `animation_color`).
            "set" if child(*b, "to").and_then(|t| child(t, "clrVal")).is_some() => match opaque(*b) {
                Some(kind) => kind,
                None => continue,
            },
            "set" => {
                let (Some(attr), Some(to)) = (attr, child(*b, "to").and_then(value)) else { continue };
                BehaviorKind::Set { attr, value: to }
            }
            // Colours that step (`valueType="clr"`, `<p:clrVal>`) are kept as they are written (see `animation_color`).
            "anim" if child(*b, "tavLst").into_iter().flat_map(elements).any(|t| child(t, "val").is_some_and(|v| child(v, "clrVal").is_some())) => match opaque(*b) {
                Some(kind) => kind,
                None => continue,
            },
            "anim" => {
                let Some(attr) = attr else { continue };
                let mut keys: Vec<(u32, String)> = child(*b, "tavLst")
                    .into_iter()
                    .flat_map(elements)
                    .filter(|t| t.tag_name().name() == "tav")
                    .filter_map(|t| Some((t.attribute("tm")?.parse().ok()?, child(t, "val").and_then(value)?)))
                    .collect();
                if keys.is_empty() {
                    if let (Some(from), Some(to)) = (b.attribute("from"), b.attribute("to")) {
                        keys = vec![(0, from.to_string()), (100_000, to.to_string())];
                    } else if let Some(by) = b.attribute("by") {
                        // `by`: from nothing to this much more (added to the value, `additive="sum"`).
                        keys = vec![(0, "0".to_string()), (100_000, by.to_string())];
                    } else if let Some(to) = b.attribute("to") {
                        // `to` alone: from the shape's own value.
                        keys = vec![(0, format!("#{attr}")), (100_000, to.to_string())];
                    }
                }
                let formula = child(*b, "tavLst").into_iter().flat_map(elements).filter_map(|t| t.attribute("fmla")).last().map(str::to_string).or_else(|| b.attribute("fmla").map(str::to_string));
                BehaviorKind::Anim { attr, keys, formula, discrete: b.attribute("calcmode") == Some("discrete") }
            }
            "animEffect" => {
                let Some(filter) = b.attribute("filter") else { continue };
                BehaviorKind::Effect { filter: filter.to_string(), enter: b.attribute("transition") != Some("out") }
            }
            "animScale" => BehaviorKind::Scale { from: pair(child(*b, "from")), to: pair(child(*b, "to")), by: pair(child(*b, "by")) },
            // A turn of the 3D view (`3d.view.rotation.y`…) is kept as written: PowerPoint's own video of it (a 360 degree turn about
            // the vertical axis over 20 s) shows the shape not moving at all (#89, 3 views of 6 times against the video).
            "animRot" if attr.as_deref().is_some_and(|a| a.starts_with("3d.")) => match opaque(*b) {
                Some(kind) => kind,
                None => continue,
            },
            "animRot" => {
                let v = |a: &str| b.attribute(a).and_then(|v| v.parse::<i32>().ok());
                BehaviorKind::Rotate { from: v("from"), to: v("to"), by: v("by") }
            }
            // Others (colours, motion…): kept as they are written, to be written back.
            _ => match opaque(*b) {
                Some(kind) => kind,
                None => continue,
            },
        };
        out.push(AnimBehavior { kind, begin, dur, time: behavior_time });
    }
    out
}

/// The points a motion path (`M 0 0 L 0.25 0 C … E`, fractions of the slide)
/// goes through after its start, in 1/100 000 of the slide. A curve (`C`,
/// `Q`) is drawn as short lines; `Z` goes back to the start. Only the
/// commands `M L C Q Z E` are read (upper case from the shape's place, lower case from where the path is).
fn path_points(path: &str) -> Option<Vec<(i32, i32)>> {
    let tokens = ibis_doc_model::animation_preset::tokenize_path(path);
    let mut points: Vec<(f64, f64)> = Vec::new();
    let mut start = (0.0, 0.0);
    let mut here = (0.0, 0.0);
    let mut i = 0;
    let num = |i: usize| -> Option<f64> { tokens.get(i)?.parse().ok() };
    let mut command = "";
    // A lower case command (`l`, `c`, `q`, `m`) takes its points relative to where the path is.
    let mut relative = false;
    while i < tokens.len() {
        let t = tokens[i];
        if t.len() == 1 && t.chars().all(|c| c.is_ascii_alphabetic()) {
            relative = t.chars().all(|c| c.is_ascii_lowercase());
            command = match t {
                "m" => "M",
                "l" => "L",
                "c" => "C",
                "q" => "Q",
                "z" => "Z",
                "e" => "E",
                other => other,
            };
            i += 1;
            if command == "Z" {
                here = start;
                points.push(here);
            }
            continue;
        }
        match command {
            "M" | "L" => {
                let mut p = (num(i)?, num(i + 1)?);
                if relative {
                    p = (here.0 + p.0, here.1 + p.1);
                }
                i += 2;
                if command == "M" && points.is_empty() {
                    start = p;
                } else {
                    points.push(p);
                }
                here = p;
                // Further pairs after an `L` are lines; after an `M`, too.
                command = "L";
            }
            "C" => {
                let o = if relative { here } else { (0.0, 0.0) };
                let (c1, c2, p) = ((o.0 + num(i)?, o.1 + num(i + 1)?), (o.0 + num(i + 2)?, o.1 + num(i + 3)?), (o.0 + num(i + 4)?, o.1 + num(i + 5)?));
                i += 6;
                for k in 1..=8 {
                    let t = k as f64 / 8.0;
                    let m = 1.0 - t;
                    let f = |a: f64, b: f64, c: f64, d: f64| m * m * m * a + 3.0 * m * m * t * b + 3.0 * m * t * t * c + t * t * t * d;
                    points.push((f(here.0, c1.0, c2.0, p.0), f(here.1, c1.1, c2.1, p.1)));
                }
                here = p;
            }
            "Q" => {
                let o = if relative { here } else { (0.0, 0.0) };
                let (c, p) = ((o.0 + num(i)?, o.1 + num(i + 1)?), (o.0 + num(i + 2)?, o.1 + num(i + 3)?));
                i += 4;
                for k in 1..=8 {
                    let t = k as f64 / 8.0;
                    let m = 1.0 - t;
                    points.push((m * m * here.0 + 2.0 * m * t * c.0 + t * t * p.0, m * m * here.1 + 2.0 * m * t * c.1 + t * t * p.1));
                }
                here = p;
            }
            _ => return None,
        }
    }
    if points.is_empty() {
        return None;
    }
    Some(points.iter().map(|&(x, y)| ((x * 100_000.0).round() as i32, (y * 100_000.0).round() as i32)).collect())
}

/// Whether the path is the one line from the start (0, 0) to its end.
fn is_single_line(points: &[(i32, i32)]) -> bool {
    points.len() == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PowerPoint's own paths (ステアステップ, バウンス) are written with lower case commands, relative to where the path is.
    #[test]
    fn relative_path_commands_are_played() {
        let p = path_points("M 0 0 l 0.036 0 l 0 0.036 l 0.036 0 E").unwrap();
        assert_eq!(p, vec![(3600, 0), (3600, 3600), (7200, 3600)]);
        let c = path_points("M 0 0 c -0.004 -0.008 -0.018 -0.016 -0.023 -0.016 c 0.031 0 0.05 0.01 0.05 0.02 E").unwrap();
        assert_eq!(c.len(), 16);
        assert_eq!(c[7], (-2300, -1600));
        assert_eq!(c[15], (2700, 400));
    }


    /// An effect on a group is one on each shape of the group: the first as
    /// the group's, the others with it.
    #[test]
    fn an_effect_on_a_group_is_one_on_each_of_its_shapes() {
        let xml = r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:timing><p:tnLst><p:par><p:cTn id="1" dur="indefinite" restart="never" nodeType="tmRoot"><p:childTnLst><p:seq concurrent="1" nextAc="seek"><p:cTn id="2" dur="indefinite" nodeType="mainSeq"><p:childTnLst><p:par><p:cTn id="3" fill="hold"><p:stCondLst><p:cond delay="indefinite"/></p:stCondLst><p:childTnLst><p:par><p:cTn id="4" fill="hold"><p:stCondLst><p:cond delay="0"/></p:stCondLst><p:childTnLst><p:par><p:cTn id="5" presetID="1" presetClass="entr" presetSubtype="0" fill="hold" nodeType="clickEffect"><p:stCondLst><p:cond delay="0"/></p:stCondLst><p:childTnLst><p:set><p:cBhvr><p:cTn id="6" dur="1" fill="hold"><p:stCondLst><p:cond delay="0"/></p:stCondLst></p:cTn><p:tgtEl><p:spTgt spid="9"/></p:tgtEl><p:attrNameLst><p:attrName>style.visibility</p:attrName></p:attrNameLst></p:cBhvr><p:to><p:strVal val="visible"/></p:to></p:set></p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn></p:seq></p:childTnLst></p:cTn></p:par></p:tnLst></p:timing></p:sld>"#;
        let doc = roxmltree::Document::parse(xml).unwrap();
        let shapes: HashMap<String, ShapeId> = [("4".to_string(), ShapeId(40))].into();
        let groups: HashMap<String, Vec<ShapeId>> = [("9".to_string(), vec![ShapeId(1), ShapeId(2), ShapeId(3)])].into();
        let no_pivots = HashMap::new();
        let effects = read(doc.root_element(), &shapes, &groups, &no_pivots, None);
        assert_eq!(effects.iter().map(|a| a.shape).collect::<Vec<_>>(), [ShapeId(1), ShapeId(2), ShapeId(3)]);
        assert_eq!(effects.iter().map(|a| a.start).collect::<Vec<_>>(), [AnimationStart::OnClick, AnimationStart::WithPrevious, AnimationStart::WithPrevious]);
        // Effects on paragraphs (`p:txEl/p:pRg`) keep their range.
        let by_paragraph = xml.replace(r#"<p:spTgt spid="9"/>"#, r#"<p:spTgt spid="4"><p:txEl><p:pRg st="2" end="3"/></p:txEl></p:spTgt>"#);
        let doc = roxmltree::Document::parse(&by_paragraph).unwrap();
        let effects = read(doc.root_element(), &shapes, &groups, &no_pivots, None);
        assert_eq!(effects.len(), 1);
        assert_eq!((effects[0].shape, effects[0].paragraph), (ShapeId(40), Some((2, 3))));
        // Effects on characters (`p:txEl/p:charRg`) and by word or letter (`p:iterate`) keep what they say.
        let by_chars = by_paragraph.replace(r#"<p:pRg st="2" end="3"/>"#, r#"<p:charRg st="4" end="9"/>"#);
        let doc = roxmltree::Document::parse(&by_chars).unwrap();
        assert_eq!(read(doc.root_element(), &shapes, &groups, &no_pivots, None)[0].chars, Some((4, 9)));
        let by_word = by_paragraph.replace(r#"</p:stCondLst><p:childTnLst><p:set>"#, r#"</p:stCondLst><p:iterate type="wd"><p:tmPct val="10000"/></p:iterate><p:childTnLst><p:set>"#).replace(r#"<p:pRg st="2" end="3"/>"#, "");
        let doc = roxmltree::Document::parse(&by_word).unwrap();
        let unit = read(doc.root_element(), &shapes, &groups, &no_pivots, None)[0].text_unit;
        assert_eq!(unit, Some((ibis_doc_model::TextUnit::Word, ibis_doc_model::UnitGap::Percent(10_000))));
        // Diagram element ST_Guid (`{00000007-0000-0000-0000-000000000000}`) decodes back to `"7"`.
        let by_dgm = xml.replace(
            r#"<p:spTgt spid="9"/>"#,
            r#"<p:spTgt spid="4"><p:graphicEl><a:dgm xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" id="{00000007-0000-0000-0000-000000000000}" bldStep="sp"/></p:graphicEl></p:spTgt>"#,
        );
        let doc = roxmltree::Document::parse(&by_dgm).unwrap();
        let dgm_el = read(doc.root_element(), &shapes, &groups, &no_pivots, None)[0].dgm_element.clone();
        assert_eq!(dgm_el, Some(ibis_doc_model::DiagramAnimationElement { id: "7".to_string(), bld_step: "sp".to_string() }));
        // A shape that is neither is no effect.
        let doc = roxmltree::Document::parse(xml).unwrap();
        assert!(read(doc.root_element(), &shapes, &HashMap::new(), &no_pivots, None).is_empty());
    }

    #[test]
    fn a_path_is_read_as_points() {
        assert_eq!(path_points("M 0 0 L 0.25 0 E"), Some(vec![(25_000, 0)]));
        // A curve is short lines, ending where it ends.
        let curve = path_points("M 0 0 C 0.1 0.1 0.2 0.2 0.3 -0.05 E").unwrap();
        assert_eq!((curve.len(), curve.last().copied()), (8, Some((30_000, -5_000))));
        // Lines one after another, and back to the start with Z.
        assert_eq!(path_points("M 0 0 L 0.1 0 L 0.1 0.2 Z E"), Some(vec![(10_000, 0), (10_000, 20_000), (0, 0)]));
        assert_eq!(path_points("M 0 0 E"), None);
        // Compact path without spaces between commands and coordinates.
        assert_eq!(path_points("M0 0L0.3 0E"), Some(vec![(30_000, 0)]));
    }

    #[test]
    fn fractional_repeat_spd_repeat_dur_group_trigger_dim_scheme_and_prefixed_timing() {
        // A1: repeatCount="500" is 0.5 times (Thousandths(500)), not 500 times.
        let xml = r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:timing><p:tnLst><p:par><p:cTn id="1" dur="indefinite" nodeType="tmRoot"><p:childTnLst><p:seq><p:cTn id="2" dur="indefinite" nodeType="mainSeq"><p:childTnLst><p:par><p:cTn id="3" fill="hold"><p:stCondLst><p:cond delay="indefinite"/></p:stCondLst><p:childTnLst><p:par><p:cTn id="4" fill="hold"><p:stCondLst><p:cond delay="0"/></p:stCondLst><p:childTnLst>
            <p:par><p:cTn id="5" presetID="8" presetClass="emph" presetSubtype="0" repeatCount="500" fill="hold" nodeType="clickEffect"><p:stCondLst><p:cond delay="0"/></p:stCondLst><p:childTnLst><p:animRot by="21600000"><p:cBhvr><p:cTn id="6" dur="2000" fill="hold"/><p:tgtEl><p:spTgt spid="9"/></p:tgtEl><p:attrNameLst><p:attrName>r</p:attrName></p:attrNameLst></p:cBhvr></p:animRot></p:childTnLst></p:cTn></p:par>
            <p:par><p:cTn id="7" presetID="8" presetClass="emph" presetSubtype="0" spd="25000" fill="hold" nodeType="withEffect"><p:stCondLst><p:cond delay="0"/></p:stCondLst><p:childTnLst><p:animRot by="21600000"><p:cBhvr><p:cTn id="8" dur="1000" fill="hold"/><p:tgtEl><p:spTgt spid="9"/></p:tgtEl><p:attrNameLst><p:attrName>r</p:attrName></p:attrNameLst></p:cBhvr></p:animRot></p:childTnLst></p:cTn></p:par>
            <p:par><p:cTn id="9" presetID="8" presetClass="emph" presetSubtype="0" repeatDur="3000" fill="hold" nodeType="withEffect"><p:stCondLst><p:cond delay="0"/></p:stCondLst><p:childTnLst><p:animRot by="21600000"><p:cBhvr><p:cTn id="10" dur="1000" fill="hold"/><p:tgtEl><p:spTgt spid="9"/></p:tgtEl><p:attrNameLst><p:attrName>r</p:attrName></p:attrNameLst></p:cBhvr></p:animRot></p:childTnLst></p:cTn></p:par>
            <p:par><p:cTn id="11" presetID="0" presetClass="path" presetSubtype="0" fill="hold" nodeType="withEffect"><p:stCondLst><p:cond delay="0"/></p:stCondLst><p:childTnLst><p:animMotion origin="layout" pathEditMode="relative"><p:cBhvr><p:cTn id="12" dur="2000" fill="hold"/><p:tgtEl><p:spTgt spid="9"/></p:tgtEl><p:attrNameLst><p:attrName>ppt_x</p:attrName><p:attrName>ppt_y</p:attrName></p:attrNameLst></p:cBhvr><p:by x="30000" y="0"/></p:animMotion></p:childTnLst></p:cTn></p:par>
            <p:par><p:cTn id="13" presetID="1" presetClass="entr" presetSubtype="0" fill="hold" nodeType="withEffect"><p:stCondLst><p:cond delay="0"/></p:stCondLst><p:childTnLst><p:set><p:cBhvr><p:cTn id="14" dur="1" fill="hold"/><p:tgtEl><p:spTgt spid="9"/></p:tgtEl><p:attrNameLst><p:attrName>style.visibility</p:attrName></p:attrNameLst></p:cBhvr><p:to><p:strVal val="visible"/></p:to></p:set></p:childTnLst><p:subTnLst><p:animClr clrSpc="rgb" dir="cw"><p:cBhvr override="childStyle"><p:cTn id="15" dur="1" fill="hold" masterRel="nextClick" afterEffect="1"/><p:tgtEl><p:spTgt spid="9"/></p:tgtEl><p:attrNameLst><p:attrName>ppt_c</p:attrName></p:attrNameLst></p:cBhvr><p:to><a:schemeClr val="accent2"/></p:to></p:animClr></p:subTnLst></p:cTn></p:par>
        </p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn></p:seq>
        <p:seq><p:cTn id="20" nodeType="interactiveSeq"><p:stCondLst><p:cond evt="onClick" delay="0"><p:tgtEl><p:spTgt spid="100"/></p:tgtEl></p:cond></p:stCondLst><p:childTnLst><p:par><p:cTn id="21" presetID="1" presetClass="entr" presetSubtype="0" fill="hold" nodeType="clickEffect"><p:stCondLst><p:cond delay="0"/></p:stCondLst><p:childTnLst><p:set><p:cBhvr><p:cTn id="22" dur="1" fill="hold"/><p:tgtEl><p:spTgt spid="9"/></p:tgtEl><p:attrNameLst><p:attrName>style.visibility</p:attrName></p:attrNameLst></p:cBhvr><p:to><p:strVal val="visible"/></p:to></p:set></p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn></p:seq>
        </p:childTnLst></p:cTn></p:par></p:tnLst></p:timing></p:sld>"#;
        let doc = roxmltree::Document::parse(xml).unwrap();
        let shapes: HashMap<String, ShapeId> = [("9".into(), ShapeId(9))].into();
        let groups: HashMap<String, Vec<ShapeId>> = [("100".into(), vec![ShapeId(10), ShapeId(11)])].into();
        let pivots: HashMap<String, (i32, i32)> = [("100".into(), (50_000, 50_000))].into();
        let theme_xml = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:clrScheme name="T"><a:accent2><a:srgbClr val="C0504D"/></a:accent2></a:clrScheme></a:themeElements></a:theme>"#;
        let theme_doc = roxmltree::Document::parse(theme_xml).unwrap();
        let theme = crate::theme::Theme::read(theme_doc.root_element());
        let colors = crate::theme::Colors::new(&theme, None, &[]);
        let effects = read(doc.root_element(), &shapes, &groups, &pivots, Some(&colors));
        assert_eq!(effects.len(), 6);
        assert_eq!(effects[0].repeat_count, AnimationRepeat::Thousandths(500));
        assert_eq!(effects[0].active_ms(), Some(1000));
        assert!((effects[0].settled_progress() - 0.5).abs() < 1e-6);
        assert_eq!(effects[1].duration, 4000);
        assert_eq!(effects[2].repeat_count, AnimationRepeat::Count(3));
        assert_eq!(effects[3].look, AnimationLook::Motion(30_000, 0));
        assert_eq!(effects[4].after_effect, AnimationAfterEffect::Dim(ibis_doc_model::Rgb(0xC0504D)));
        assert!(effects[5].trigger.matches_click(ShapeId(10)));
        assert!(effects[5].trigger.matches_click(ShapeId(11)));
        assert!(!effects[5].trigger.matches_click(ShapeId(9)));

        // B5: alternate namespace prefix `pp:` in `pp:timing` keeps opaque colour and motion behaviours.
        let pp_xml = r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><pp:timing xmlns:pp="http://schemas.openxmlformats.org/presentationml/2006/main"><pp:tnLst><pp:par><pp:cTn id="1" dur="indefinite" nodeType="tmRoot"><pp:childTnLst><pp:seq><pp:cTn id="2" dur="indefinite" nodeType="mainSeq"><pp:childTnLst><pp:par><pp:cTn id="3" fill="hold"><pp:stCondLst><pp:cond delay="indefinite"/></pp:stCondLst><pp:childTnLst><pp:par><pp:cTn id="4" fill="hold"><pp:stCondLst><pp:cond delay="0"/></pp:stCondLst><pp:childTnLst>
            <pp:par><pp:cTn id="5" presetID="1" presetClass="emph" presetSubtype="2" fill="hold" nodeType="clickEffect"><pp:stCondLst><pp:cond delay="0"/></pp:stCondLst><pp:childTnLst><pp:animClr clrSpc="rgb" dir="cw"><pp:cBhvr><pp:cTn id="6" dur="1500" fill="hold"/><pp:tgtEl><pp:spTgt spid="9"/></pp:tgtEl><pp:attrNameLst><pp:attrName>fillcolor</pp:attrName></pp:attrNameLst></pp:cBhvr><pp:to><a:srgbClr val="FF0000"/></pp:to></pp:animClr></pp:childTnLst></pp:cTn></pp:par>
            <pp:par><pp:cTn id="7" presetID="0" presetClass="emph" presetSubtype="0" fill="hold" nodeType="withEffect"><pp:stCondLst><pp:cond delay="0"/></pp:stCondLst><pp:childTnLst><pp:animMotion origin="layout" path="M 0 0 L 0.18 0 E"><pp:cBhvr><pp:cTn id="8" dur="2000" fill="hold"/><pp:tgtEl><pp:spTgt spid="9"/></pp:tgtEl><pp:attrNameLst><pp:attrName>ppt_x</pp:attrName><pp:attrName>ppt_y</pp:attrName></pp:attrNameLst></pp:cBhvr></pp:animMotion></pp:childTnLst></pp:cTn></pp:par>
        </pp:childTnLst></pp:cTn></pp:par></pp:childTnLst></pp:cTn></pp:par></pp:childTnLst></pp:cTn></pp:seq></pp:childTnLst></pp:cTn></pp:par></pp:tnLst></pp:timing></p:sld>"#;
        let pp_doc = roxmltree::Document::parse(pp_xml).unwrap();
        let pp_effects = read(pp_doc.root_element(), &shapes, &groups, &pivots, Some(&colors));
        assert_eq!(pp_effects.len(), 2);
        assert_eq!(pp_effects[0].look, AnimationLook::Preset);
        assert_eq!(pp_effects[0].behaviors.len(), 1);
        assert_eq!(pp_effects[1].look, AnimationLook::Preset);
        assert_eq!(pp_effects[1].behaviors.len(), 1);
        let b = ibis_doc_model::animation_preset::PptBox { x: 0.5, y: 0.5, w: 0.2, h: 0.2 };
        let st_clr = ibis_doc_model::animation_preset::sample(&pp_effects[0].behaviors, 1500.0, b);
        assert_eq!(st_clr.colors.len(), 1);
        let st_mot = ibis_doc_model::animation_preset::sample(&pp_effects[1].behaviors, 2000.0, b);
        assert!((st_mot.motion.unwrap().0 - 0.18).abs() < 1e-9);
    }
}
