//! Task classifier: natural-language user task → [`TaskType`].
//!
//! Pure keyword/score heuristics — deterministic, unit-testable, no network.
//! The selected [`TaskType`] drives [`crate::skill::SkillRegistry`] lookup.

/// Coarse task categories the runtime knows how to organize work around.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskType {
    BugFix,
    Feature,
    Test,
    Refactor,
    CodeReview,
    Docs,
    /// General work mode tag (docs/summaries/Q&A). Never produced by
    /// [`classify`] — selected explicitly for Work mode runs.
    Work,
}

impl TaskType {
    /// Stable wire/label form (`bug-fix`, `code-review`, …).
    pub fn label(self) -> &'static str {
        match self {
            Self::BugFix => "bug-fix",
            Self::Feature => "feature",
            Self::Test => "test",
            Self::Refactor => "refactor",
            Self::CodeReview => "code-review",
            Self::Docs => "docs",
            Self::Work => "work",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "bug-fix" | "bugfix" | "bug_fix" => Some(Self::BugFix),
            "feature" => Some(Self::Feature),
            "test" | "testing" => Some(Self::Test),
            "refactor" => Some(Self::Refactor),
            "code-review" | "review" | "code_review" => Some(Self::CodeReview),
            "docs" | "documentation" => Some(Self::Docs),
            "work" => Some(Self::Work),
            _ => None,
        }
    }

    pub const ALL: [TaskType; 7] = [
        Self::BugFix,
        Self::Feature,
        Self::Test,
        Self::Refactor,
        Self::CodeReview,
        Self::Docs,
        Self::Work,
    ];
}

impl std::fmt::Display for TaskType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// Scored keyword rules. Order matters only via scores; ties break by rule order.
fn score(task: &str, patterns: &[&str]) -> u32 {
    let lower = task.to_ascii_lowercase();
    patterns
        .iter()
        .map(|p| {
            let p = p.to_ascii_lowercase();
            if lower.contains(&p) {
                // Longer, more specific patterns weigh more.
                1 + (p.chars().count() as u32 / 4)
            } else {
                0
            }
        })
        .sum()
}

/// The margin between the winning rule score and the runner-up that still
/// counts as a clear decision.
///
/// This is a *policy constant*, not a measurement: the rule scores are
/// keyword weights, so a difference of 4 points says "the keywords leaned
/// this way", never "the request is 4% more likely to be a refactor". The
/// value is still to be evaluated against the frozen corpus before the
/// decision component is ever allowed to act on the result.
pub const AMBIGUITY_MARGIN: u32 = 4;

/// The route label vocabulary shared by the frozen corpus, the decision
/// assistant's choices and the trace UI.
///
/// **Why this exists:** the runtime category ([`TaskType`]) and the route label
/// are not the same set. `work` is only ever selected explicitly for Work-mode
/// runs and never by [`classify`], while a request the local rules have no
/// evidence about needs a label that does not pretend to be a category. The
/// scorer, the assistant and the UI must read one vocabulary, not three.
pub const ROUTE_LABELS: [&str; 7] = [
    "bug-fix",
    "feature",
    "test",
    "refactor",
    "code-review",
    "docs",
    "unknown",
];

/// Where a classification came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassificationSource {
    /// The message asks for a document as the deliverable.
    DocumentOutput,
    /// The keyword rules decided.
    Keywords,
    /// There was no message to classify.
    Empty,
}

/// The rule evidence behind a classification.
///
/// The scores are keyword weights. They are reported so a reader can see
/// *why* a category won — they are not probabilities and must never be
/// presented as confidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassificationEvidence {
    /// The category the local rules produced.
    pub baseline: TaskType,
    pub source: ClassificationSource,
    /// Every category's rule score, highest first.
    pub scores: Vec<(TaskType, u32)>,
    /// The winning score.
    pub top_score: u32,
    /// The score of the category that came second.
    pub runner_up_score: u32,
    /// How many categories matched at least one keyword.
    pub hit_count: usize,
    /// True when no keyword matched at all and the default was used.
    pub default_fallback: bool,
    /// True when the rules did not decide clearly: no keyword hit, a tie at
    /// the top, or a margin below [`AMBIGUITY_MARGIN`].
    pub ambiguous: bool,
}

impl ClassificationEvidence {
    /// The route label for this reading.
    ///
    /// `unknown` is emitted only in the one state that has no evidence behind
    /// it: the keyword rules matched nothing and the message is not a plain
    /// "save this answer as a document" instruction. It is never invented to
    /// fill a gap. [`classify`] keeps returning the additive `feature` default
    /// in that state, so skill selection and the budget family are unchanged —
    /// the label reports the uncertainty instead of hiding it.
    pub fn route_label(&self) -> &'static str {
        if self.default_fallback && self.source != ClassificationSource::DocumentOutput {
            return "unknown";
        }
        self.baseline.label()
    }
}

/// Classify a natural-language task into one of the six task types.
///
/// Deterministic heuristic: the highest-scoring category wins; ties fall back
/// to [`TaskType::Feature`] (the common additive default).
pub fn classify(message: &str) -> TaskType {
    classify_with_evidence(message).baseline
}

/// Classify a task and report the rule evidence behind the decision.
///
/// [`classify`] stays the compatibility entry point and keeps returning
/// exactly the same category; this function adds the *reason* so a caller
/// can tell a clear decision from a coin flip.
pub fn classify_with_evidence(message: &str) -> ClassificationEvidence {
    let m = message.trim();
    if m.is_empty() {
        return ClassificationEvidence {
            baseline: TaskType::Feature,
            source: ClassificationSource::Empty,
            scores: Vec::new(),
            top_score: 0,
            runner_up_score: 0,
            hit_count: 0,
            default_fallback: true,
            ambiguous: true,
        };
    }
    let scores = score_all(m);
    // "把这份结论保存到 docs/x.md" is a file task; "新增导出为 Markdown 的入口"
    // is a code task that happens to name its output format. The document rule
    // may only decide when nothing in the message asks for the repository to
    // change — otherwise a format word outranks the action the user asked for.
    if is_document_output(m) && !asks_for_code_change(&scores) {
        return ClassificationEvidence {
            baseline: TaskType::Docs,
            source: ClassificationSource::DocumentOutput,
            scores: Vec::new(),
            top_score: 0,
            runner_up_score: 0,
            hit_count: 0,
            default_fallback: false,
            ambiguous: false,
        };
    }
    // Same traversal as the original implementation: strictly greater wins,
    // so a tie keeps the category that comes first in the rule order.
    let mut best = TaskType::Feature;
    let mut best_score = 0u32;
    for (kind, s) in &scores {
        if *s > best_score {
            best_score = *s;
            best = *kind;
        }
    }
    let mut ranked: Vec<(TaskType, u32)> = scores.to_vec();
    // Stable sort keeps the rule order for equal scores, so the reported
    // order matches the order the winner was chosen in.
    ranked.sort_by(|a, b| b.1.cmp(&a.1));
    let runner_up_score = ranked
        .iter()
        .skip(1)
        .map(|(_, score)| *score)
        .find(|score| *score > 0)
        .unwrap_or(0);
    let hit_count = scores.iter().filter(|(_, score)| *score > 0).count();
    let tied_at_top = ranked
        .iter()
        .skip(1)
        .find(|(_, score)| *score > 0)
        .is_some_and(|(_, score)| *score == best_score);
    let default_fallback = hit_count == 0;
    let ambiguous = default_fallback
        || tied_at_top
        || best_score.saturating_sub(runner_up_score) < AMBIGUITY_MARGIN;
    ClassificationEvidence {
        baseline: best,
        source: ClassificationSource::Keywords,
        scores: ranked,
        top_score: best_score,
        runner_up_score,
        hit_count,
        default_fallback,
        ambiguous,
    }
}

/// Categories whose deliverable is a change to the repository.
///
/// `CodeReview` and `Docs` are deliberately excluded: their deliverable is a
/// report or a document, so they must not veto the document-output rule.
fn produces_code_change(kind: TaskType) -> bool {
    matches!(
        kind,
        TaskType::Feature | TaskType::BugFix | TaskType::Refactor | TaskType::Test
    )
}

/// Does the message ask for the repository to change? Used only to decide
/// whether the document-output rule may short-circuit the keyword scores.
fn asks_for_code_change(scores: &[(TaskType, u32)]) -> bool {
    scores
        .iter()
        .any(|(kind, score)| *score > 0 && produces_code_change(*kind))
}

/// The rule scores for every keyword-driven category, in rule order.
fn score_all(m: &str) -> [(TaskType, u32); 6] {
    let scores = [
        (
            TaskType::CodeReview,
            score(
                m,
                &[
                    "code review",
                    "review the",
                    "review this",
                    "review patch",
                    "review pr",
                    "review mr",
                    "please review",
                    "审查",
                    "评审",
                    "评估",
                    "code-review",
                    "review",
                ],
            ),
        ),
        (
            TaskType::BugFix,
            score(
                m,
                &[
                    "fix the",
                    "fix this",
                    "fix bug",
                    "bug fix",
                    "bugfix",
                    "panic",
                    "crash",
                    "stack trace",
                    "failing test",
                    "broken",
                    "regression",
                    "修复",
                    "报错",
                    "崩溃",
                    "异常",
                    "故障",
                    "错误日志",
                    "不工作",
                    "fix",
                    "bug",
                ],
            ),
        ),
        (
            TaskType::Refactor,
            score(
                m,
                &[
                    "refactor",
                    "restructure",
                    "clean up",
                    "cleanup",
                    "rename",
                    "extract function",
                    "extract module",
                    "拆分",
                    "重构",
                    "整理代码",
                    "重命名",
                ],
            ),
        ),
        (
            TaskType::Test,
            score(
                m,
                &[
                    "add tests",
                    "add test",
                    "write tests",
                    "write test",
                    "unit test",
                    "integration test",
                    "test coverage",
                    "test case",
                    "补测试",
                    "写测试",
                    "加测试",
                    "测试用例",
                    "单元测试",
                    "覆盖率",
                    "测试",
                    "test",
                ],
            ),
        ),
        (
            TaskType::Docs,
            score(
                m,
                &[
                    "readme",
                    "documentation",
                    "changelog",
                    "docs",
                    "docstring",
                    "api docs",
                    "更新文档",
                    "补充文档",
                    "使用文档",
                    "注释",
                    "文档",
                    "说明文档",
                ],
            ),
        ),
        (
            TaskType::Feature,
            score(
                m,
                &[
                    "implement",
                    "add support",
                    "add a",
                    "add an",
                    "new feature",
                    "create a",
                    "build a",
                    "实现",
                    "新增",
                    "添加",
                    "支持",
                    "做一个",
                    "开发",
                ],
            ),
        ),
    ];

    scores
}

/// Saving an answer is a file task, even when its filename contains "架构".
pub(crate) fn is_document_output(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    if [
        "如何",
        "怎么",
        "怎样",
        "how to",
        "修复",
        "实现",
        "重构",
        "fix ",
        "implement",
        "refactor",
    ]
    .iter()
    .any(|word| lower.contains(word))
    {
        return false;
    }
    let output = [
        "输出到",
        "保存到",
        "保存为",
        "存成",
        "写入",
        "写到",
        "写进",
        "导出",
        "另存为",
        "落盘",
        "save ",
        "export ",
        "write ",
        "output to",
    ]
    .iter()
    .any(|word| lower.contains(word));
    let document = [".md", "markdown", "文档", "docs/", "docs下", "docs 下"]
        .iter()
        .any(|word| lower.contains(word));
    output && document
}

#[cfg(test)]
mod tests {
    use super::*;

    // Two representative tasks per task_type — both must classify correctly.
    #[test]
    fn classifies_bug_fix_examples() {
        assert_eq!(
            classify("Fix the panic when opening an empty project"),
            TaskType::BugFix
        );
        assert_eq!(classify("修复登录接口 500 报错"), TaskType::BugFix);
    }

    #[test]
    fn classifies_feature_examples() {
        assert_eq!(
            classify("Add a dark mode toggle to settings"),
            TaskType::Feature
        );
        assert_eq!(classify("实现导出 CSV 的功能"), TaskType::Feature);
    }

    #[test]
    fn classifies_test_examples() {
        assert_eq!(
            classify("Add unit tests for the task classifier"),
            TaskType::Test
        );
        assert_eq!(classify("给 plan 模块补测试用例"), TaskType::Test);
    }

    #[test]
    fn classifies_refactor_examples() {
        assert_eq!(
            classify("Refactor the verify module into smaller functions"),
            TaskType::Refactor
        );
        assert_eq!(classify("重构上下文扫描逻辑"), TaskType::Refactor);
    }

    #[test]
    fn classifies_code_review_examples() {
        assert_eq!(
            classify("Please review the patch in crates/agent"),
            TaskType::CodeReview
        );
        assert_eq!(classify("审查一下这个 PR 的改动"), TaskType::CodeReview);
    }

    #[test]
    fn assessment_requests_are_reviews_not_features() {
        assert_eq!(classify("评估这次改动的风险点"), TaskType::CodeReview);
        assert_eq!(classify("评估一下这次改动的可维护性"), TaskType::CodeReview);
        assert_eq!(
            classify("对k2k-rust的能力做一个详细的评估"),
            TaskType::CodeReview
        );
    }

    #[test]
    fn classifies_docs_examples() {
        assert_eq!(
            classify("Update the README installation section"),
            TaskType::Docs
        );
        assert_eq!(classify("补充 API 使用文档"), TaskType::Docs);
    }

    #[test]
    fn labels_roundtrip_through_parse() {
        for kind in TaskType::ALL {
            assert_eq!(TaskType::parse(kind.label()), Some(kind));
        }
        assert_eq!(TaskType::parse("nope"), None);
    }

    #[test]
    fn empty_task_defaults_to_feature() {
        assert_eq!(classify(""), TaskType::Feature);
        assert_eq!(classify("   "), TaskType::Feature);
    }

    #[test]
    fn the_compatibility_entry_point_agrees_with_the_evidence() {
        for message in [
            "Fix the panic when opening an empty project",
            "Add a dark mode toggle to settings",
            "Add unit tests for the task classifier",
            "Refactor the verify module into smaller functions",
            "Please review the patch in crates/agent",
            "Update the README installation section",
            "修复登录接口 500 报错",
            "实现导出 CSV 的功能",
            "",
        ] {
            assert_eq!(
                classify(message),
                classify_with_evidence(message).baseline,
                "{message}"
            );
        }
    }

    #[test]
    fn a_clear_keyword_decision_is_not_ambiguous() {
        let evidence = classify_with_evidence("Please review the patch in crates/agent");
        assert_eq!(evidence.baseline, TaskType::CodeReview);
        assert_eq!(evidence.source, ClassificationSource::Keywords);
        assert!(!evidence.ambiguous, "{evidence:?}");
        assert!(!evidence.default_fallback);
        assert!(evidence.top_score > evidence.runner_up_score);
    }

    #[test]
    fn a_message_without_any_keyword_is_ambiguous() {
        let evidence = classify_with_evidence("把那个东西弄好");
        assert_eq!(evidence.baseline, TaskType::Feature);
        assert!(evidence.default_fallback);
        assert!(evidence.ambiguous, "{evidence:?}");
        assert_eq!(evidence.hit_count, 0);
    }

    #[test]
    fn a_tie_at_the_top_is_ambiguous() {
        // "review" lands on CodeReview, "test" on Test; neither leads.
        let evidence = classify_with_evidence("review test");
        assert!(evidence.ambiguous, "{evidence:?}");
    }

    #[test]
    fn a_document_request_is_decided_by_the_document_rule() {
        let evidence = classify_with_evidence("保存到 docs/架构.md");
        assert_eq!(evidence.baseline, TaskType::Docs);
        assert_eq!(evidence.source, ClassificationSource::DocumentOutput);
        assert!(!evidence.ambiguous, "{evidence:?}");
    }

    #[test]
    fn a_feature_that_names_its_output_format_is_not_a_document_task() {
        // FR-B3 / FR-B4 / AC-B1: "导出为 Markdown" names the format of what the
        // code must produce; the user asked for an entry point in the
        // repository. The document rule may only decide when nothing in the
        // message asks the project to change.
        let evidence = classify_with_evidence("新增导出为 Markdown 的入口");
        assert_eq!(evidence.baseline, TaskType::Feature, "{evidence:?}");
        assert_eq!(evidence.route_label(), "feature");
        assert_ne!(evidence.source, ClassificationSource::DocumentOutput);
        // The veto is one-directional: a plain save is still a document task.
        assert_eq!(
            classify_with_evidence("保存到 docs/架构.md").route_label(),
            "docs"
        );
    }

    #[test]
    fn route_label_names_uncertainty_instead_of_borrowing_the_default() {
        // FR-B2 / AC-B2: `unknown` comes only from the no-evidence state, and
        // only in the label. `classify` keeps its additive default, so skill
        // selection and the budget family do not move.
        for message in ["把那个东西弄好", "do the usual thing"] {
            let evidence = classify_with_evidence(message);
            assert_eq!(evidence.route_label(), "unknown", "{message}");
            assert!(evidence.ambiguous, "{message}");
            assert_eq!(classify(message), TaskType::Feature, "{message}");
        }
        // A reading with evidence is never labelled unknown.
        assert_eq!(
            classify_with_evidence("修复登录接口 500 报错").route_label(),
            "bug-fix"
        );
    }

    #[test]
    fn an_empty_message_is_ambiguous_and_falls_back() {
        let evidence = classify_with_evidence("   ");
        assert_eq!(evidence.baseline, TaskType::Feature);
        assert_eq!(evidence.source, ClassificationSource::Empty);
        assert!(evidence.default_fallback);
        assert!(evidence.ambiguous, "{evidence:?}");
    }
}
