import { CircleCheck } from "lucide-react";
import { AgentTrace } from "./AgentTrace";
import { ChangedFilesSummary } from "./ChangedFilesSummary";
import type { Reply } from "./trace";

type Props = {
  time: string;
  reply: Reply;
  /** True while this turn is the one being generated. */
  running: boolean;
  onViewFiles: () => void;
};

export function AssistantReply({ time, reply, running, onViewFiles }: Props) {
  return (
    <article className="reply">
      <div className="msg-head">
        <span className="avatar avatar--kodo">K</span>
        <span className="msg-author">Kodo</span>
        {time && <span className="msg-time">{time}</span>}
      </div>

      {running && <p className="reply-working">正在处理您的请求...</p>}
      {reply.interrupted && <p className="reply-interrupted">这次运行中断了，最后一步没有完成。</p>}

      <AgentTrace steps={reply.steps} />

      {reply.final && (
        <div className="final">
          <p className="final-text">{reply.final}</p>

          {reply.checks.length > 0 && (
            <section className="checks">
              <h3 className="checks-head">
                <CircleCheck size={15} strokeWidth={1.9} />
                <span>检查结果</span>
              </h3>
              <ul className="checks-list">
                {reply.checks.map((check) => (
                  <li key={check}>{check}</li>
                ))}
              </ul>
            </section>
          )}
        </div>
      )}

      {reply.files > 0 && (
        <ChangedFilesSummary
          files={reply.files}
          added={reply.added}
          removed={reply.removed}
          onView={onViewFiles}
        />
      )}
    </article>
  );
}
