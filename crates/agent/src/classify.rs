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
            _ => None,
        }
    }

    pub const ALL: [TaskType; 6] = [
        Self::BugFix,
        Self::Feature,
        Self::Test,
        Self::Refactor,
        Self::CodeReview,
        Self::Docs,
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

/// Classify a natural-language task into one of the six task types.
///
/// Deterministic heuristic: the highest-scoring category wins; ties fall back
/// to [`TaskType::Feature`] (the common additive default).
pub fn classify(message: &str) -> TaskType {
    let m = message.trim();
    if m.is_empty() {
        return TaskType::Feature;
    }

    let scores: [(TaskType, u32); 6] = [
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

    let mut best = TaskType::Feature;
    let mut best_score = 0u32;
    for (kind, s) in scores {
        if s > best_score {
            best_score = s;
            best = kind;
        }
    }
    best
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
}
