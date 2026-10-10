import { Database, FolderOpen, Info, Keyboard, SquareArrowOutUpRight, Zap } from "lucide-react";
import type { ReactNode } from "react";
import { T } from "../i18n";
import type { PluginType } from "../routes";

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

export function HelpOverview({ pluginType }: { pluginType?: PluginType }) {
  if (pluginType) {
    const help = T.page.pluginHelp[pluginType];
    return (
      <>
        <HelpCard icon={<Info size={14} strokeWidth={1.7} />} title={help.title}>
          <p className="ins-note">{help.body}</p>
        </HelpCard>
        <HelpCard icon={<FolderOpen size={14} strokeWidth={1.7} />} title="使用范围">
          <p className="ins-note">{help.scope}</p>
        </HelpCard>
        <HelpCard icon={<Keyboard size={14} strokeWidth={1.7} />} title={T.inspector.helpShortcuts}>
          <p className="ins-note">{T.inspector.helpShortcutsBody}</p>
        </HelpCard>
      </>
    );
  }
  return (
    <>
      <HelpCard icon={<Info size={14} strokeWidth={1.7} />} title={T.inspector.helpAbout}>
        <p className="ins-note">
          归档会话会从侧边栏隐藏，但记录仍完整保留。可在「归档」页面选择会话并恢复。
        </p>
      </HelpCard>

      <HelpCard icon={<Database size={14} strokeWidth={1.7} />} title={T.inspector.helpStorage}>
        <p className="ins-note">
          项目列表、会话与设置使用本机 append-only 日志（macOS：
          <code>~/Library/Application Support/Kodo</code>）。API Key 单独存放在 credentials.log（权限 0600），
          不会写入 settings.log。代码保留在你的本地工作区；只有配置了 Provider 时才会把对话与笔记发送给你自己的模型端点。
        </p>
      </HelpCard>

      <HelpCard icon={<Zap size={14} strokeWidth={1.7} />} title={T.inspector.helpActions}>
        <div className="quick-action">
          <FolderOpen size={15} strokeWidth={1.7} />
          <span>
            <span className="quick-title">{T.inspector.helpDataDir}</span>
            <span className="quick-sub">~/Library/Application Support/Kodo</span>
          </span>
        </div>
        <div className="quick-action">
          <SquareArrowOutUpRight size={15} strokeWidth={1.7} />
          <span>
            <span className="quick-title">{T.inspector.helpDocs}</span>
            <span className="quick-sub">{T.inspector.helpDocsSub}</span>
          </span>
        </div>
      </HelpCard>

      <HelpCard icon={<Keyboard size={14} strokeWidth={1.7} />} title={T.inspector.helpShortcuts}>
        <p className="ins-note">{T.inspector.helpShortcutsBody}</p>
      </HelpCard>

      <HelpCard icon={<Info size={14} strokeWidth={1.7} />} title={T.inspector.helpMore}>
        <p className="ins-note">查看仓库 README 与 docs/design/，默认权限为「请求批准」。</p>
      </HelpCard>
    </>
  );
}
