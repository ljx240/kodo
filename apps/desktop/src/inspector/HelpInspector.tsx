import { Database, FolderOpen, Info, SquareArrowOutUpRight, Zap } from "lucide-react";
import type { ReactNode } from "react";

/** Static help card — no chevron: a disclosure glyph with no toggle would be a dead affordance. */
function HelpCard({ icon, title, children }: { icon: ReactNode; title: string; children: ReactNode }) {
  return (
    <section className="ins-section">
      <header className="ins-section-head">
        <span className="ins-section-icon">{icon}</span>
        <h3>{title}</h3>
        <span className="spacer" />
      </header>
      <div className="ins-body">{children}</div>
    </section>
  );
}

export function HelpOverview() {
  return (
    <>
      <HelpCard icon={<Info size={14} strokeWidth={1.7} />} title="About archive">
        <p className="ins-note">
          归档会话会从侧边栏隐藏，但日志文件仍完整保留。可在 Archive 页通过 Inspector 恢复。
        </p>
      </HelpCard>

      <HelpCard icon={<Database size={14} strokeWidth={1.7} />} title="Where your data is stored">
        <p className="ins-note">
          项目列表、会话与设置使用本机 append-only 日志（macOS：
          <code>~/Library/Application Support/Kodo</code>）。API Key 单独存放在 credentials.log（权限 0600），
          不会写入 settings.log。代码保留在你的本地工作区；只有配置了 Provider 时才会把对话与笔记发送给你自己的模型端点。
        </p>
      </HelpCard>

      <HelpCard icon={<Zap size={14} strokeWidth={1.7} />} title="Quick actions">
        <div className="quick-action">
          <FolderOpen size={15} strokeWidth={1.7} />
          <span>
            <span className="quick-title">Local data directory</span>
            <span className="quick-sub">~/Library/Application Support/Kodo</span>
          </span>
        </div>
        <div className="quick-action">
          <SquareArrowOutUpRight size={15} strokeWidth={1.7} />
          <span>
            <span className="quick-title">Open documentation</span>
            <span className="quick-sub">See README in the repository</span>
          </span>
        </div>
      </HelpCard>

      <HelpCard icon={<Info size={14} strokeWidth={1.7} />} title="Need more help?">
        <p className="ins-note">查看仓库 README 与 docs/design/，默认权限为「请求批准」。</p>
      </HelpCard>
    </>
  );
}
