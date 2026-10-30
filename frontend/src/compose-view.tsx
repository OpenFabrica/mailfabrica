import { t } from "./i18n";
import { useEffect, useRef, useState } from "react";
import { EditorContent, useEditor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import {
  TextStyle,
  FontSize,
  BackgroundColor,
} from "@tiptap/extension-text-style";
import Color from "@tiptap/extension-color";
import {
  Bold,
  Italic,
  Underline,
  List,
  ListOrdered,
  Link,
  Paperclip,
  Send,
  Save,
  Sparkles,
  Trash2,
  X,
  AlignLeft,
  Quote,
} from "lucide-react";
import type { Compose } from "./types";
import type { SessionUI } from "./use-session";
import { execute } from "./api";
import { Dialog, IconButton } from "./components";
import { EmailContent, useEmailTheme } from "./email-content";
import { useDismiss } from "./use-dismiss";
import { RecipientInput } from "./recipients";
// ComposeView 将编辑内容通过 compose.setHtml 命令写入 Go；引用区不可编辑。
export function ComposeView({
  compose,
  ui,
  onClose,
  standalone = false,
}: {
  compose: Compose;
  ui: SessionUI;
  onClose: () => void;
  standalone?: boolean;
}) {
  const { run, state } = ui;
  const en = state?.prefs.language === "en";
  const [fields, setFields] = useState({
    to: compose.to,
    cc: compose.cc,
    bcc: compose.bcc,
    subject: compose.subject,
    account: compose.account,
  });
  const [cc, setCC] = useState(Boolean(compose.cc || compose.bcc));
  const [instruction, setInstruction] = useState("");
  const [ai, setAI] = useState(false);
  const [link, setLink] = useState<string | null>(null);
  const linkRoot = useDismiss<HTMLFormElement>(link !== null, () =>
    setLink(null),
  );
  const dark = useEmailTheme(state?.prefs.theme);
  const upload = useRef<HTMLInputElement>(null);
  const pending = useRef<Promise<unknown>>(Promise.resolve());
  const current = useRef(compose);
  current.current = compose;
  const ownEdit = useRef(false);
  const editor = useEditor({
    immediatelyRender: false,
    extensions: [StarterKit, TextStyle, Color, FontSize, BackgroundColor],
    content: compose.html,
    editable: !compose.sending,
    onUpdate: ({ editor }) => {
      ownEdit.current = true;
      const html = editor.getHTML();
      pending.current = pending.current
        .then(() => execute("compose.setHtml", { id: compose.id, html }))
        .catch((e) => ui.setError(String(e)));
    },
  });
  useEffect(() => {
    if (!editor || editor.isDestroyed || !editor.schema) return;
    editor.setEditable(!compose.sending, false);
    if (!ownEdit.current && editor.getHTML() !== compose.html)
      editor.commands.setContent(compose.html, { emitUpdate: false });
    ownEdit.current = false;
  }, [compose.html, compose.sending, editor]);
  const update = (key: string, value: string) => {
    setFields((f) => ({ ...f, [key]: value }));
    pending.current = pending.current
      .then(() => execute("compose.set", { id: compose.id, [key]: value }))
      .catch((e) => ui.setError(String(e)));
  };
  const action = async (
    command: string,
    params: Record<string, unknown> = {},
  ) => {
    await pending.current;
    return run(command, { id: compose.id, ...params });
  };
  const close = () => {
    if (compose.sending) return;
    void action("compose.discard").then(onClose);
  };
  const content = (
    <>
      <div className="compose-fields">
        <label>
          <span>{en ? "From" : t("发件人")}</span>
          <select
            value={fields.account}
            onChange={(e) => update("account", e.target.value)}
          >
            {state?.accounts.map((a) => (
              <option value={a.id} key={a.id}>
                {a.name} &lt;{a.email}&gt;
              </option>
            ))}
          </select>
        </label>
        <label>
          <span>{en ? "To" : t("收件人")}</span>
          <RecipientInput
            label={en ? "To" : t("收件人")}
            value={fields.to}
            onChange={(value) => update("to", value)}
            en={en}
            disabled={compose.sending}
            autoFocus
          />
          <button
            onClick={() => setCC(!cc)}
            type="button"
            className="text-button muted"
          >
            {en ? "Cc / Bcc" : t("抄送 / 密送")}
          </button>
        </label>
        {cc && (
          <>
            <label>
              <span>{en ? "Cc" : t("抄送")}</span>
              <RecipientInput
                label={en ? "Cc" : t("抄送")}
                value={fields.cc}
                onChange={(value) => update("cc", value)}
                en={en}
                disabled={compose.sending}
              />
            </label>
            <label>
              <span>{en ? "Bcc" : t("密送")}</span>
              <RecipientInput
                label={en ? "Bcc" : t("密送")}
                value={fields.bcc}
                onChange={(value) => update("bcc", value)}
                en={en}
                disabled={compose.sending}
              />
            </label>
          </>
        )}
        <label>
          <span>{en ? "Subject" : t("主题")}</span>
          <input
            aria-label={en ? "Subject" : t("主题")}
            value={fields.subject}
            onChange={(e) => update("subject", e.target.value)}
          />
        </label>
      </div>
      <div className="format-bar">
        <select
          aria-label={t("字号")}
          className="font-size"
          defaultValue="14px"
          onChange={(e) =>
            editor?.chain().focus().setFontSize(e.target.value).run()
          }
        >
          {[
            ["12px", "小"],
            ["14px", "正常"],
            ["18px", "大"],
            ["24px", "特大"],
          ].map(([value, label]) => (
            <option key={value} value={value}>
              {t(label)}
            </option>
          ))}
        </select>

        <IconButton
          label={t("加粗")}
          className={editor?.isActive("bold") ? "active" : ""}
          onClick={() => editor?.chain().focus().toggleBold().run()}
        >
          <Bold />
        </IconButton>
        <IconButton
          label={t("斜体")}
          onClick={() => editor?.chain().focus().toggleItalic().run()}
        >
          <Italic />
        </IconButton>
        <IconButton
          label={t("下划线")}
          onClick={() => editor?.chain().focus().toggleUnderline().run()}
        >
          <Underline />
        </IconButton>
        <IconButton
          label={t("删除线")}
          onClick={() => editor?.chain().focus().toggleStrike().run()}
        >
          <span style={{ textDecoration: "line-through" }}>S</span>
        </IconButton>
        <IconButton
          label={t("插入链接")}
          onClick={() =>
            setLink(String(editor?.getAttributes("link").href || ""))
          }
        >
          <Link />
        </IconButton>
        <span className="divider" />
        <IconButton
          label={t("项目列表")}
          onClick={() => editor?.chain().focus().toggleBulletList().run()}
        >
          <List />
        </IconButton>
        <IconButton
          label={t("编号列表")}
          onClick={() => editor?.chain().focus().toggleOrderedList().run()}
        >
          <ListOrdered />
        </IconButton>
        <IconButton
          label={t("引用")}
          onClick={() => editor?.chain().focus().toggleBlockquote().run()}
        >
          <Quote />
        </IconButton>
        <label className="color-control" title={t("文字颜色")}>
          <span>A</span>
          <input
            type="color"
            aria-label={t("文字颜色")}
            onChange={(e) =>
              editor?.chain().focus().setColor(e.target.value).run()
            }
          />
        </label>
        <IconButton
          label={t("清除格式")}
          onClick={() =>
            editor?.chain().focus().unsetAllMarks().clearNodes().run()
          }
        >
          <AlignLeft />
        </IconButton>
        <button
          className="text-button muted"
          onClick={() => {
            void action("compose.setFormat", { rich: !compose.rich });
          }}
        >
          {compose.rich
            ? en
              ? "Rich text"
              : t("富文本")
            : en
              ? "Plain text"
              : t("纯文本")}
        </button>
        <span className="grow" />
        <button
          className={`text-button accent ${ai ? "active" : ""}`}
          onClick={() => setAI(!ai)}
        >
          <Sparkles size={14} />
          {en ? "AI assistant" : t("AI 助手")}
        </button>
      </div>
      {link !== null && (
        <form
          className="link-editor"
          ref={linkRoot}
          onSubmit={(e) => {
            e.preventDefault();
            if (!link) {
              editor?.chain().focus().unsetLink().run();
              setLink(null);
              return;
            }
            try {
              const parsed = new URL(link);
              if (!["http:", "https:", "mailto:"].includes(parsed.protocol))
                throw new Error(t("链接协议不支持"));
              editor
                ?.chain()
                .focus()
                .extendMarkRange("link")
                .setLink({ href: link })
                .run();
              setLink(null);
            } catch (e) {
              ui.setError(String(e));
            }
          }}
        >
          <input
            autoFocus
            aria-label={t("链接地址")}
            placeholder="https://"
            value={link}
            onChange={(e) => setLink(e.target.value)}
          />
          <button>{t("应用")}</button>
          <button type="button" onClick={() => setLink(null)}>
            {t("取消")}
          </button>
        </form>
      )}
      {ai && (
        <div className="ai-prompt">
          <input
            aria-label={t("AI 写信要求")}
            value={instruction}
            placeholder={
              en
                ? "What would you like to write?"
                : t("想写些什么，或需要怎样修改？")
            }
            onChange={(e) => setInstruction(e.target.value)}
          />
          <select id="ai-action" aria-label={t("AI 操作")} defaultValue="write">
            <option value="write">{t("起草")}</option>
            <option value="polish">{t("润色")}</option>
            <option value="formal">{t("更正式")}</option>
            <option value="friendly">{t("更友好")}</option>
            <option value="shorter">{t("更短")}</option>
            <option value="longer">{t("更长")}</option>
            <option value="translate">{t("翻译")}</option>
          </select>
          <button
            disabled={compose.aiBusy}
            onClick={() => {
              void action("ai.compose", {
                instruction,
                action:
                  (document.querySelector("#ai-action") as HTMLSelectElement)
                    ?.value || "write",
              });
            }}
          >
            {compose.aiBusy ? t("正在生成…") : t("生成")}
          </button>
        </div>
      )}
      {compose.aiSuggestion && (
        <div className="ai-suggestion">
          <p>{compose.aiSuggestion}</p>
          <div>
            <button
              onClick={() => {
                ownEdit.current = false;
                void action("ai.accept", { mode: "replace" });
              }}
            >
              {t("替换正文")}
            </button>
            <button
              onClick={() => {
                ownEdit.current = false;
                void action("ai.accept", { mode: "append" });
              }}
            >
              {t("接在后面")}
            </button>
            <button
              onClick={() => {
                void action("ai.dismiss");
              }}
            >
              {t("放弃")}
            </button>
          </div>
        </div>
      )}
      {(compose.error || compose.aiError) && (
        <div className="inline-error">{compose.error || compose.aiError}</div>
      )}
      <div className="compose-editor">
        {compose.rich ? (
          <EditorContent editor={editor} />
        ) : (
          <textarea
            className="plain-editor"
            aria-label={t("邮件正文")}
            defaultValue={compose.body}
            onChange={(e) => {
              void action("compose.set", { body: e.target.value });
            }}
          />
        )}
        {compose.quote && (
          <section className="compose-quote" aria-label={t("原邮件")}>
            <div className="compose-quote-heading">
              <span>
                <Quote size={14} />
                {t("原邮件")}
              </span>
              <IconButton
                label={t("移除原文引用")}
                onClick={() => {
                  void action("compose.removeQuote");
                }}
              >
                <X />
              </IconButton>
            </div>
            <EmailContent
              title={t("原邮件引用")}
              html={compose.quote.html || null}
              text={compose.quote.text}
              remoteImages={Boolean(state?.prefs.remoteImages)}
              dark={dark}
              onLinkError={ui.setError}
              quoted
            />
          </section>
        )}
      </div>
      <div className="compose-attachments">
        {compose.attachments.map((a, i) => (
          <span key={`${a.name}-${i}`}>
            <Paperclip size={13} />
            {a.name}
            <IconButton
              label={`移除 ${a.name}`}
              onClick={() => {
                void action("compose.removeAttachment", { index: i });
              }}
            >
              <X />
            </IconButton>
          </span>
        ))}
      </div>
      <footer className="compose-footer">
        <button
          className="primary"
          disabled={compose.sending}
          onClick={() => {
            void action("compose.send");
          }}
        >
          <Send size={14} />
          {compose.sending
            ? en
              ? "Working…"
              : t("处理中…")
            : en
              ? "Send"
              : t("发送")}
        </button>
        <button
          disabled={compose.sending}
          onClick={() => {
            void action("compose.saveDraft");
          }}
        >
          <Save size={14} />
          {en ? "Save draft" : t("存草稿")}
        </button>
        <IconButton
          label={t("添加附件")}
          onClick={() => upload.current?.click()}
        >
          <Paperclip />
        </IconButton>
        <input
          ref={upload}
          type="file"
          multiple
          hidden
          onChange={(e) => {
            const files = [...(e.target.files || [])];
            void (async () => {
              for (const file of files) {
                if (file.size > 50 * 1024 * 1024) {
                  ui.setError(t("附件不能超过 50 MiB"));
                  continue;
                }
                const base64 = await new Promise<string>((resolve, reject) => {
                  const reader = new FileReader();
                  reader.onload = () =>
                    resolve(String(reader.result).split(",")[1]);
                  reader.onerror = reject;
                  reader.readAsDataURL(file);
                });
                await action("compose.attach", { name: file.name, base64 });
              }
            })();
            e.target.value = "";
          }}
        />
        <span className="grow" />
        <span className="muted">
          {compose.dirty ? (en ? "Unsaved changes" : t("有未保存的修改")) : ""}
        </span>
        <IconButton
          label={t("丢弃草稿")}
          disabled={compose.sending}
          onClick={close}
        >
          <Trash2 />
        </IconButton>
      </footer>
    </>
  );
  return standalone ? (
    <main className="compose-standalone">{content}</main>
  ) : (
    <Dialog title={en ? "New message" : t("写邮件")} onClose={close} wide>
      {content}
    </Dialog>
  );
}
