import { useState } from "react";
import { Languages, LoaderCircle, X } from "lucide-react";
import type { SessionUI } from "./use-session";

// MailTranslation 按邮件键隔离译文，语言选择变化时保留结果的真实语言标签。
export function MailTranslation({
  ui,
  messageKey,
}: {
  ui: SessionUI;
  messageKey: string;
}) {
  const en = ui.state?.prefs.language === "en";
  const [language, setLanguage] = useState(en ? "en" : "zh-CN");
  const [visible, setVisible] = useState(true);
  const translation = ui.state?.aiTranslation;
  const current = translation?.key === messageKey ? translation : null;
  const languages: Record<string, string> = {
    "zh-CN": en ? "Simplified Chinese" : "简体中文",
    "zh-TW": en ? "Traditional Chinese" : "繁体中文",
    en: en ? "English" : "英文",
    ja: en ? "Japanese" : "日文",
    ko: en ? "Korean" : "韩文",
  };
  return (
    <section
      className="mail-translation"
      aria-label={en ? "Email translation" : "邮件翻译"}
    >
      <div className="translation-toolbar">
        <Languages size={15} aria-hidden="true" />
        <select
          aria-label={en ? "Translation language" : "翻译目标语言"}
          value={language}
          onChange={(e) => setLanguage(e.target.value)}
        >
          {Object.entries(languages).map(([value, label]) => (
            <option key={value} value={value}>
              {label}
            </option>
          ))}
        </select>
        <button
          className="text-button accent"
          disabled={!!translation?.busy}
          data-testid="reader.translate"
          onClick={() => {
            setVisible(true);
            void ui.run("ai.translate", { key: messageKey, language });
          }}
        >
          {current?.busy && <LoaderCircle size={14} className="spin" />}
          {current?.busy
            ? en
              ? "Translating…"
              : "正在翻译…"
            : en
              ? "AI translate"
              : "AI 翻译"}
        </button>
        {current && !visible && (
          <button className="text-button" onClick={() => setVisible(true)}>
            {en ? "Show translation" : "显示译文"}
          </button>
        )}
      </div>
      {current && visible && (
        <aside
          className="translation-result"
          aria-label={en ? "AI translation" : "AI 译文"}
        >
          <div className="translation-toolbar">
            <strong>
              {en ? "AI translation" : "AI 译文"} ·{" "}
              {languages[current.language]}
            </strong>
            <button
              className="icon-button"
              aria-label={en ? "Hide translation" : "收起译文"}
              onClick={() => setVisible(false)}
            >
              <X size={14} />
            </button>
          </div>
          {current.busy && (
            <p role="status">{en ? "Translating email…" : "正在翻译邮件…"}</p>
          )}
          {current.error && (
            <p role="alert" className="inline-error">
              {current.error}
            </p>
          )}
          {current.text && (
            <div className="summary-text" data-testid="reader.translationText">
              {current.text}
            </div>
          )}
        </aside>
      )}
    </section>
  );
}
