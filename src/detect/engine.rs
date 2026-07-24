//! Manifest rule engine, reimplemented to be compatible with herdr's
//! detection engine version 3 (`src/detect/manifest.rs`, Apache-2.0; see
//! NOTICE). The vendored TOMLs under `manifests/` are consumed verbatim, so
//! every semantic here — region slicing, gate combination, winner selection —
//! must match herdr's behavior exactly.

use regex::Regex;
use serde::Deserialize;

pub const MANIFEST_ENGINE_VERSION: u32 = 3;

/// Screen-derived agent state as resolved by manifest rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineState {
    Idle,
    Working,
    Blocked,
    Unknown,
}

impl EngineState {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "idle" => Some(Self::Idle),
            "working" => Some(Self::Working),
            "blocked" => Some(Self::Blocked),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Working => "working",
            Self::Blocked => "blocked",
            Self::Unknown => "unknown",
        }
    }
}

/// Input to the engine: the pane's visible screen text plus OSC-derived
/// strings. Pass "" when a source is unavailable.
#[derive(Debug, Clone, Copy)]
pub struct DetectionInput<'a> {
    pub screen: &'a str,
    pub osc_title: &'a str,
    pub osc_progress: &'a str,
}

/// Result of evaluating a manifest against one input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detection {
    pub state: EngineState,
    /// Id of the winning rule; `None` means the known-agent Idle fallback.
    pub rule_id: Option<String>,
    /// Winning rule was `skip_state_update`: discard this detection and keep
    /// the pane's previous state.
    pub skip: bool,
    /// The winning rule carried the visible flag matching its state.
    pub visible: bool,
}

pub const KNOWN_AGENT_IDLE_FALLBACK: Detection = Detection {
    state: EngineState::Idle,
    rule_id: None,
    skip: false,
    visible: false,
};

/// Per-rule evaluation record for `--explain`.
#[derive(Debug, Clone)]
pub struct RuleTrace {
    pub rule_id: String,
    pub priority: i32,
    pub region: String,
    pub state: EngineState,
    pub matched: bool,
    pub region_text: String,
}

// ---------------------------------------------------------------------------
// TOML schema (deny_unknown_fields doubles as upstream drift detection)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentManifest {
    pub id: String,
    #[allow(dead_code)] // schema completeness; not needed at runtime
    version: Option<String>,
    min_engine_version: Option<u32>,
    #[serde(rename = "updated_at")]
    #[allow(dead_code)]
    _updated_at: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    aliases: Vec<String>,
    #[serde(default)]
    rules: Vec<ManifestRule>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestRule {
    id: String,
    state: String,
    #[serde(default)]
    priority: i32,
    #[serde(default = "default_region")]
    region: String,
    #[serde(default)]
    skip_state_update: bool,
    #[serde(default)]
    visible_idle: bool,
    #[serde(default)]
    visible_blocker: bool,
    #[serde(default)]
    visible_working: bool,
    #[serde(default)]
    contains: Vec<String>,
    #[serde(default)]
    regex: Vec<String>,
    #[serde(default)]
    line_regex: Vec<String>,
    #[serde(default)]
    all: Vec<ManifestGate>,
    #[serde(default)]
    any: Vec<ManifestGate>,
    #[serde(default)]
    not: Vec<ManifestGate>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestGate {
    #[serde(default)]
    contains: Vec<String>,
    #[serde(default)]
    regex: Vec<String>,
    #[serde(default)]
    line_regex: Vec<String>,
    #[serde(default)]
    all: Vec<ManifestGate>,
    #[serde(default)]
    any: Vec<ManifestGate>,
    #[serde(default)]
    not: Vec<ManifestGate>,
}

fn default_region() -> String {
    "whole_recent".to_string()
}

impl AgentManifest {
    /// Region specs used by this manifest's rules (for coverage checks).
    #[allow(dead_code)] // used by the manifest region-coverage test
    pub fn rule_regions(&self) -> impl Iterator<Item = &str> {
        self.rules.iter().map(|rule| rule.region.as_str())
    }
}

// ---------------------------------------------------------------------------
// Compilation
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct CompiledManifest {
    pub id: String,
    rules: Vec<CompiledRule>,
}

#[derive(Debug)]
struct CompiledRule {
    id: String,
    state: EngineState,
    priority: i32,
    region: String,
    skip_state_update: bool,
    visible_idle: bool,
    visible_blocker: bool,
    visible_working: bool,
    gate: CompiledGate,
}

#[derive(Debug)]
struct CompiledGate {
    contains_lower: Vec<String>,
    regex: Vec<Regex>,
    line_regex: Vec<Regex>,
    all: Vec<CompiledGate>,
    any: Vec<CompiledGate>,
    not: Vec<CompiledGate>,
}

pub fn parse_manifest(text: &str) -> Result<AgentManifest, String> {
    toml::from_str(text).map_err(|err| err.to_string())
}

pub fn compile(manifest: &AgentManifest) -> Result<CompiledManifest, String> {
    if let Some(min_engine) = manifest.min_engine_version
        && min_engine > MANIFEST_ENGINE_VERSION
    {
        return Err(format!(
            "manifest {} requires engine version {min_engine}, this engine is {MANIFEST_ENGINE_VERSION}",
            manifest.id
        ));
    }

    let mut rules = Vec::with_capacity(manifest.rules.len());
    for rule in &manifest.rules {
        let state = EngineState::parse(&rule.state)
            .ok_or_else(|| format!("rule {}: unknown state {:?}", rule.id, rule.state))?;
        if rule.skip_state_update {
            if state != EngineState::Unknown {
                return Err(format!(
                    "rule {}: skip_state_update requires state \"unknown\"",
                    rule.id
                ));
            }
            if rule.visible_idle || rule.visible_blocker || rule.visible_working {
                return Err(format!(
                    "rule {}: skip_state_update cannot combine with visible flags",
                    rule.id
                ));
            }
        }
        rules.push(CompiledRule {
            id: rule.id.clone(),
            state,
            priority: rule.priority,
            region: rule.region.clone(),
            skip_state_update: rule.skip_state_update,
            visible_idle: rule.visible_idle,
            visible_blocker: rule.visible_blocker,
            visible_working: rule.visible_working,
            gate: compile_gate(
                &rule.contains,
                &rule.regex,
                &rule.line_regex,
                &rule.all,
                &rule.any,
                &rule.not,
            )
            .map_err(|err| format!("rule {}: {err}", rule.id))?,
        });
    }

    Ok(CompiledManifest {
        id: manifest.id.clone(),
        rules,
    })
}

fn compile_gate(
    contains: &[String],
    regex: &[String],
    line_regex: &[String],
    all: &[ManifestGate],
    any: &[ManifestGate],
    not: &[ManifestGate],
) -> Result<CompiledGate, String> {
    let compile_patterns = |patterns: &[String]| -> Result<Vec<Regex>, String> {
        patterns
            .iter()
            .map(|pattern| Regex::new(pattern).map_err(|err| format!("bad regex: {err}")))
            .collect()
    };
    let compile_gates = |gates: &[ManifestGate]| -> Result<Vec<CompiledGate>, String> {
        gates
            .iter()
            .map(|gate| {
                compile_gate(
                    &gate.contains,
                    &gate.regex,
                    &gate.line_regex,
                    &gate.all,
                    &gate.any,
                    &gate.not,
                )
            })
            .collect()
    };

    Ok(CompiledGate {
        contains_lower: contains.iter().map(|s| s.to_lowercase()).collect(),
        regex: compile_patterns(regex)?,
        line_regex: compile_patterns(line_regex)?,
        all: compile_gates(all)?,
        any: compile_gates(any)?,
        not: compile_gates(not)?,
    })
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

pub fn evaluate(manifest: &CompiledManifest, input: DetectionInput) -> Detection {
    evaluate_inner(manifest, input, None)
}

pub fn explain(manifest: &CompiledManifest, input: DetectionInput) -> (Detection, Vec<RuleTrace>) {
    let mut traces = Vec::new();
    let detection = evaluate_inner(manifest, input, Some(&mut traces));
    (detection, traces)
}

fn evaluate_inner(
    manifest: &CompiledManifest,
    input: DetectionInput,
    mut traces: Option<&mut Vec<RuleTrace>>,
) -> Detection {
    let mut winner: Option<&CompiledRule> = None;

    for rule in &manifest.rules {
        let text = region(input, &rule.region);
        let matched = rule_matches(rule, text);
        if let Some(traces) = traces.as_deref_mut() {
            traces.push(RuleTrace {
                rule_id: rule.id.clone(),
                priority: rule.priority,
                region: rule.region.clone(),
                state: rule.state,
                matched,
                region_text: text.to_string(),
            });
        }
        if !matched {
            continue;
        }
        match winner {
            // Strictly-greater replacement: on a tie the earliest rule wins.
            Some(previous) if previous.priority >= rule.priority => {}
            _ => winner = Some(rule),
        }
    }

    let Some(rule) = winner else {
        return KNOWN_AGENT_IDLE_FALLBACK;
    };

    let visible = match rule.state {
        EngineState::Idle => rule.visible_idle,
        EngineState::Blocked => rule.visible_blocker,
        EngineState::Working => rule.visible_working,
        EngineState::Unknown => false,
    };

    Detection {
        state: rule.state,
        rule_id: Some(rule.id.clone()),
        skip: rule.skip_state_update,
        visible,
    }
}

fn rule_matches(rule: &CompiledRule, text: &str) -> bool {
    // Lowercased once per rule evaluation; nested gates share the region.
    let lower_text = text.to_lowercase();
    gate_matches(&rule.gate, text, &lower_text)
}

fn gate_matches(gate: &CompiledGate, text: &str, lower_text: &str) -> bool {
    gate.contains_lower
        .iter()
        .all(|needle| lower_text.contains(needle))
        && gate.regex.iter().all(|regex| regex.is_match(text))
        && gate
            .line_regex
            .iter()
            .all(|regex| text.lines().any(|line| regex.is_match(line)))
        && gate
            .all
            .iter()
            .all(|nested| gate_matches(nested, text, lower_text))
        && (gate.any.is_empty()
            || gate
                .any
                .iter()
                .any(|nested| gate_matches(nested, text, lower_text)))
        && !gate
            .not
            .iter()
            .any(|nested| gate_matches(nested, text, lower_text))
}

// ---------------------------------------------------------------------------
// Regions
// ---------------------------------------------------------------------------

fn region<'a>(input: DetectionInput<'a>, spec: &str) -> &'a str {
    let trimmed = spec.trim();
    // OSC regions source from their dedicated fields, not the screen.
    match trimmed {
        "osc_title" => return input.osc_title,
        "osc_progress" => return input.osc_progress,
        _ => {}
    }
    let content = input.screen;
    match trimmed {
        "whole_recent" => content,
        "after_last_prompt_marker" => after_last_prompt_marker(content),
        "prompt_box_body" => prompt_box_body(content).unwrap_or(""),
        "after_last_horizontal_rule" => after_last_horizontal_rule(content),
        _ => {
            if let Some(count) = region_count(trimmed, "bottom_lines") {
                return bottom_lines(content, count);
            }
            if let Some(count) = region_count(trimmed, "bottom_non_empty_lines") {
                return bottom_non_empty_lines(content, count);
            }
            if let Some(count) = region_count(trimmed, "top_non_empty_lines") {
                return top_non_empty_lines(content, count);
            }
            ""
        }
    }
}

/// Region specs implemented by this engine. Kept in sync with `region()`; the
/// coverage test in `manifests.rs` asserts every region used by the vendored
/// manifests appears here, so an upstream refresh cannot silently dead-letter
/// rules.
#[allow(dead_code)] // used by the manifest region-coverage test
pub fn region_is_supported(spec: &str) -> bool {
    let trimmed = spec.trim();
    matches!(
        trimmed,
        "osc_title"
            | "osc_progress"
            | "whole_recent"
            | "after_last_prompt_marker"
            | "prompt_box_body"
            | "after_last_horizontal_rule"
    ) || region_count(trimmed, "bottom_lines").is_some()
        || region_count(trimmed, "bottom_non_empty_lines").is_some()
        || region_count(trimmed, "top_non_empty_lines").is_some()
}

fn region_count(spec: &str, name: &str) -> Option<usize> {
    spec.strip_prefix(name)
        .and_then(|rest| rest.strip_prefix('('))
        .and_then(|rest| rest.strip_suffix(')'))
        .and_then(|count| count.parse::<usize>().ok())
}

fn line_start_offset(content: &str, lines: &[&str], index: usize) -> usize {
    lines[..index.min(lines.len())]
        .iter()
        .map(|line| line.len() + 1)
        .sum::<usize>()
        .min(content.len())
}

fn slice_from_line_index<'a>(content: &'a str, lines: &[&str], index: usize) -> &'a str {
    &content[line_start_offset(content, lines, index)..]
}

fn bottom_lines(content: &str, count: usize) -> &str {
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(count);
    slice_from_line_index(content, &lines, start)
}

fn bottom_non_empty_lines(content: &str, count: usize) -> &str {
    let lines: Vec<&str> = content.lines().collect();
    let Some(start_index) = lines
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, line)| !line.trim().is_empty())
        .take(count)
        .last()
        .map(|(index, _)| index)
    else {
        return "";
    };
    slice_from_line_index(content, &lines, start_index)
}

fn top_non_empty_lines(content: &str, count: usize) -> &str {
    let lines: Vec<&str> = content.lines().collect();
    let Some(end_index) = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .take(count)
        .last()
        .map(|(index, _)| index)
    else {
        return "";
    };
    &content[..line_start_offset(content, &lines, end_index + 1)]
}

fn codex_prompt_line(line: &str) -> bool {
    line == "›" || line.starts_with("› ")
}

fn after_last_prompt_marker(content: &str) -> &str {
    let lines: Vec<&str> = content.lines().collect();
    let Some(index) = lines.iter().rposition(|line| codex_prompt_line(line)) else {
        return content;
    };
    slice_from_line_index(content, &lines, index + 1)
}

fn after_last_horizontal_rule(content: &str) -> &str {
    let mut last_rule_end = 0usize;
    let mut offset = 0usize;
    for line in content.lines() {
        let next_offset = offset + line.len() + 1;
        if is_horizontal_rule(line) {
            last_rule_end = next_offset.min(content.len());
        }
        offset = next_offset;
    }
    &content[last_rule_end..]
}

fn prompt_box_body(content: &str) -> Option<&str> {
    let lines: Vec<&str> = content.lines().collect();
    let top = prompt_box_top_border_index(&lines)?;
    let start = line_start_offset(content, &lines, top + 1);
    let end_index = lines[top + 1..]
        .iter()
        .position(|line| is_horizontal_rule(line))
        .map(|relative| top + 1 + relative)
        .unwrap_or(lines.len());
    let end = line_start_offset(content, &lines, end_index);
    Some(&content[start..end.max(start)])
}

fn prompt_box_top_border_index(lines: &[&str]) -> Option<usize> {
    let mut border_count = 0;
    for index in (0..lines.len()).rev() {
        if is_horizontal_rule(lines[index]) {
            border_count += 1;
            if border_count == 2 {
                return Some(index);
            }
        }
    }
    None
}

fn is_horizontal_rule(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }

    let rule_chars = trimmed.chars().take_while(|&ch| ch == '─').count();
    if rule_chars == 0 {
        return false;
    }

    let rule_bytes = trimmed
        .char_indices()
        .nth(rule_chars)
        .map(|(index, _)| index)
        .unwrap_or(trimmed.len());
    let suffix = trimmed[rule_bytes..].trim_start();

    suffix.is_empty() || rule_chars >= 3
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn screen_input(screen: &str) -> DetectionInput<'_> {
        DetectionInput {
            screen,
            osc_title: "",
            osc_progress: "",
        }
    }

    fn compile_toml(text: &str) -> CompiledManifest {
        compile(&parse_manifest(text).expect("manifest should parse")).expect("should compile")
    }

    // ---- Regions ----

    #[test]
    fn whole_recent_is_the_entire_screen() {
        let input = screen_input("a\nb\nc");
        assert_eq!(region(input, "whole_recent"), "a\nb\nc");
    }

    #[test]
    fn osc_regions_read_dedicated_fields() {
        let input = DetectionInput {
            screen: "screen",
            osc_title: "the title",
            osc_progress: "4;0",
        };
        assert_eq!(region(input, "osc_title"), "the title");
        assert_eq!(region(input, "osc_progress"), "4;0");
    }

    #[test]
    fn bottom_non_empty_lines_includes_interleaved_blanks() {
        let content = "top\nsecond\n\nthird\n\nlast";
        // Last 2 non-empty lines are "third" and "last"; slice starts at
        // "third" and keeps the blank between them.
        assert_eq!(
            region(screen_input(content), "bottom_non_empty_lines(2)"),
            "third\n\nlast"
        );
    }

    #[test]
    fn bottom_non_empty_lines_of_blank_content_is_empty() {
        assert_eq!(
            region(screen_input("\n\n \n"), "bottom_non_empty_lines(3)"),
            ""
        );
    }

    #[test]
    fn bottom_lines_counts_physical_lines() {
        assert_eq!(
            region(screen_input("a\nb\nc\nd"), "bottom_lines(2)"),
            "c\nd"
        );
    }

    #[test]
    fn top_non_empty_lines_slices_from_start() {
        let content = "\nfirst\nsecond\nthird";
        assert_eq!(
            region(screen_input(content), "top_non_empty_lines(1)"),
            "\nfirst\n"
        );
    }

    #[test]
    fn top_non_empty_lines_of_blank_content_is_empty() {
        assert_eq!(region(screen_input("\n \n"), "top_non_empty_lines(1)"), "");
    }

    #[test]
    fn after_last_prompt_marker_variants() {
        assert_eq!(
            region(
                screen_input("out\n› command\ntail"),
                "after_last_prompt_marker"
            ),
            "tail"
        );
        assert_eq!(
            region(screen_input("out\n›\ntail"), "after_last_prompt_marker"),
            "tail"
        );
        // "›x" without a space is NOT a prompt marker.
        assert_eq!(
            region(screen_input("out\n›x\ntail"), "after_last_prompt_marker"),
            "out\n›x\ntail"
        );
    }

    #[test]
    fn horizontal_rule_detection_paths() {
        assert!(is_horizontal_rule("───"));
        assert!(is_horizontal_rule("  ─  "));
        assert!(is_horizontal_rule("──── some label"));
        assert!(!is_horizontal_rule("── label")); // run < 3 with suffix
        assert!(!is_horizontal_rule(""));
        assert!(!is_horizontal_rule("text"));
    }

    #[test]
    fn after_last_horizontal_rule_slices_after_rule_line() {
        let content = "before\n────\nafter\nmore";
        assert_eq!(
            region(screen_input(content), "after_last_horizontal_rule"),
            "after\nmore"
        );
        // No rule: whole content.
        assert_eq!(
            region(screen_input("a\nb"), "after_last_horizontal_rule"),
            "a\nb"
        );
    }

    #[test]
    fn prompt_box_body_is_between_the_two_bottom_rules() {
        let content = "history\n────\n❯ type here\n────\nhints";
        assert_eq!(
            region(screen_input(content), "prompt_box_body"),
            "❯ type here\n"
        );
    }

    #[test]
    fn prompt_box_body_requires_two_rules() {
        assert_eq!(region(screen_input("a\n────\nb"), "prompt_box_body"), "");
    }

    #[test]
    fn unknown_region_is_empty() {
        assert_eq!(region(screen_input("content"), "no_such_region"), "");
        assert!(!region_is_supported("no_such_region"));
        assert!(region_is_supported("bottom_non_empty_lines(5)"));
    }

    // ---- Gate combination ----

    const GATE_MANIFEST: &str = r#"
id = "test"

[[rules]]
id = "combo"
state = "blocked"
contains = ["Do You Want"]
regex = ['esc to \w+']
any = [{ contains = ["yes"] }, { contains = ["ok"] }]
not = [{ contains = ["forbidden"] }]
"#;

    #[test]
    fn contains_is_case_insensitive_and_regex_is_not() {
        let manifest = compile_toml(GATE_MANIFEST);
        // "DO YOU WANT" matches case-insensitively; regex needs exact case.
        let hit = evaluate(&manifest, screen_input("DO YOU WANT? esc to cancel yes"));
        assert_eq!(hit.state, EngineState::Blocked);
        assert_eq!(hit.rule_id.as_deref(), Some("combo"));

        let miss = evaluate(&manifest, screen_input("DO YOU WANT? ESC TO CANCEL yes"));
        assert_eq!(miss.rule_id, None, "regex must be case-sensitive");
    }

    #[test]
    fn any_gate_requires_one_hit_and_not_gate_vetoes() {
        let manifest = compile_toml(GATE_MANIFEST);
        let no_any = evaluate(&manifest, screen_input("do you want? esc to cancel"));
        assert_eq!(no_any.rule_id, None, "empty any candidates must miss");

        let vetoed = evaluate(
            &manifest,
            screen_input("do you want? esc to cancel yes forbidden"),
        );
        assert_eq!(vetoed.rule_id, None, "not gate must veto");
    }

    #[test]
    fn nested_gates_combine() {
        let manifest = compile_toml(
            r#"
id = "test"

[[rules]]
id = "nested"
state = "working"
all = [{ any = [{ contains = ["alpha"], not = [{ contains = ["beta"] }] }] }]
"#,
        );
        assert_eq!(
            evaluate(&manifest, screen_input("alpha"))
                .rule_id
                .as_deref(),
            Some("nested")
        );
        assert_eq!(
            evaluate(&manifest, screen_input("alpha beta")).rule_id,
            None
        );
    }

    #[test]
    fn line_regex_matches_any_single_line() {
        let manifest = compile_toml(
            r#"
id = "test"

[[rules]]
id = "lined"
state = "idle"
line_regex = ['^\s*❯']
"#,
        );
        assert_eq!(
            evaluate(&manifest, screen_input("text\n  ❯ prompt"))
                .rule_id
                .as_deref(),
            Some("lined")
        );
        // Anchored pattern must not match mid-line across the whole blob.
        assert_eq!(
            evaluate(&manifest, screen_input("text ❯ prompt")).rule_id,
            None
        );
    }

    // ---- Winner selection ----

    const PRIORITY_MANIFEST: &str = r#"
id = "test"

[[rules]]
id = "first_low"
state = "idle"
priority = 10
contains = ["x"]

[[rules]]
id = "second_same"
state = "working"
priority = 10
contains = ["x"]

[[rules]]
id = "third_high"
state = "blocked"
priority = 20
contains = ["y"]
"#;

    #[test]
    fn tie_goes_to_earliest_rule() {
        let manifest = compile_toml(PRIORITY_MANIFEST);
        let detection = evaluate(&manifest, screen_input("x"));
        assert_eq!(detection.rule_id.as_deref(), Some("first_low"));
        assert_eq!(detection.state, EngineState::Idle);
    }

    #[test]
    fn strictly_higher_priority_wins_regardless_of_order() {
        let manifest = compile_toml(PRIORITY_MANIFEST);
        let detection = evaluate(&manifest, screen_input("x y"));
        assert_eq!(detection.rule_id.as_deref(), Some("third_high"));
        assert_eq!(detection.state, EngineState::Blocked);
    }

    #[test]
    fn no_match_falls_back_to_known_agent_idle() {
        let manifest = compile_toml(PRIORITY_MANIFEST);
        let detection = evaluate(&manifest, screen_input("nothing relevant"));
        assert_eq!(detection, KNOWN_AGENT_IDLE_FALLBACK);
    }

    // ---- skip_state_update & visible flags ----

    #[test]
    fn skip_state_update_winner_reports_skip() {
        let manifest = compile_toml(
            r#"
id = "test"

[[rules]]
id = "viewer"
state = "unknown"
priority = 100
skip_state_update = true
contains = ["transcript"]

[[rules]]
id = "idle"
state = "idle"
visible_idle = true
contains = ["transcript"]
"#,
        );
        let detection = evaluate(&manifest, screen_input("showing transcript"));
        assert!(detection.skip);
        assert_eq!(detection.rule_id.as_deref(), Some("viewer"));
    }

    #[test]
    fn skip_state_update_validation_rejects_non_unknown_state() {
        let manifest = parse_manifest(
            r#"
id = "test"

[[rules]]
id = "bad"
state = "idle"
skip_state_update = true
contains = ["x"]
"#,
        )
        .expect("parses");
        assert!(compile(&manifest).is_err());
    }

    #[test]
    fn visible_flag_propagates_only_for_matching_state() {
        let manifest = compile_toml(
            r#"
id = "test"

[[rules]]
id = "vis"
state = "working"
visible_working = true
contains = ["spin"]

[[rules]]
id = "novis"
state = "blocked"
visible_working = true
contains = ["blocked"]
"#,
        );
        assert!(evaluate(&manifest, screen_input("spin")).visible);
        assert!(!evaluate(&manifest, screen_input("blocked")).visible);
    }

    #[test]
    fn explain_traces_every_rule() {
        let manifest = compile_toml(PRIORITY_MANIFEST);
        let (detection, traces) = explain(&manifest, screen_input("x y"));
        assert_eq!(detection.rule_id.as_deref(), Some("third_high"));
        assert_eq!(traces.len(), 3);
        assert!(traces.iter().all(|trace| trace.region == "whole_recent"));
        assert_eq!(
            traces.iter().filter(|trace| trace.matched).count(),
            3,
            "all three rules match the combined input"
        );
    }
}
