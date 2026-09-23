import { Puzzle, Sparkles } from "lucide-react";
import { BUILTIN_SKILLS } from "../conversation/Composer";
import { T } from "../i18n";

export function SkillsPage() {
  return (
    <main className="main">
      <header className="page-head">
        <span className="page-head-mark"><Sparkles size={18} strokeWidth={1.7} /></span>
        <div className="page-head-text">
          <h1>{T.page.skillsHead}</h1>
          <p>{T.page.skillsSubtitle}</p>
        </div>
      </header>
      <div className="scroll">
        <div className="page-inner page-inner--wide">
          <section className="skills-card" aria-label={T.page.skillsCard}>
            {BUILTIN_SKILLS.map((skill) => (
              <div className="skill-row" key={skill.id}>
                <Puzzle size={17} strokeWidth={1.7} />
                <span className="skill-row-label">{skill.label}</span>
                <code>/{skill.id}</code>
              </div>
            ))}
          </section>
        </div>
      </div>
    </main>
  );
}
