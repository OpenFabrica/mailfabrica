import { useDismiss } from "./use-dismiss";
import { useEffect, useId, useRef, useState } from "react";
import { X } from "lucide-react";
import { execute } from "./api";

// splitRecipients 只在引号和尖括号之外拆分列表，保留带逗号的联系人名称。
export function splitRecipients(value: string) {
  const parts: string[] = [];
  let token = "",
    quoted = false,
    escaped = false,
    angle = 0;
  for (const char of value) {
    if (escaped) {
      token += char;
      escaped = false;
      continue;
    }
    if (quoted && char === "\\") {
      token += char;
      escaped = true;
      continue;
    }
    if (char === '"') quoted = !quoted;
    if (!quoted && char === "<") angle++;
    if (!quoted && char === ">") angle = Math.max(0, angle - 1);
    if (!quoted && angle === 0 && /[,;\n\r，；]/.test(char)) {
      if (token.trim()) parts.push(token.trim());
      token = "";
    } else token += char;
  }
  if (token.trim()) parts.push(token.trim());
  return parts;
}

// recipientEmail 提取常用地址格式；最终收发合法性仍由 Go 地址解析器校验。
export function recipientEmail(value: string) {
  const token = value.trim().replace(/^mailto:/i, "");
  const match = token.match(/<([^<>]+)>$/);
  const email = (match?.[1] || token).trim();
  return /^[^\s@<>(),;:"\\]+@[^\s@<>(),;:"\\]+$/.test(email) ? email : null;
}

// RecipientInput 为收件人、抄送和密送共用多地址标签、粘贴拆分和联系人匹配。
export function RecipientInput({
  label,
  value,
  onChange,
  en,
  disabled = false,
  autoFocus = false,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  en: boolean;
  disabled?: boolean;
  autoFocus?: boolean;
}) {
  const [tokens, setTokens] = useState(() =>
    splitRecipients(value).filter((v) => recipientEmail(v)),
  );
  const [draft, setDraft] = useState(() =>
    splitRecipients(value)
      .filter((v) => !recipientEmail(v))
      .join(", "),
  );
  const [open, setOpen] = useState(false),
    [active, setActive] = useState(0),
    [invalid, setInvalid] = useState(false);
  const popupRoot = useDismiss(open, () => setOpen(false));
  const last = useRef(value),
    input = useRef<HTMLInputElement>(null),
    id = useId();
  useEffect(() => {
    if (value === last.current) return;
    const parts = splitRecipients(value);
    setTokens(parts.filter((v) => recipientEmail(v)));
    setDraft(parts.filter((v) => !recipientEmail(v)).join(", "));
    last.current = value;
  }, [value]);
  const [matched, setMatched] = useState<{
    query: string;
    values: { name: string; email: string }[];
  }>({ query: "", values: [] });
  const [lookupError, setLookupError] = useState("");
  useEffect(() => {
    if (!draft.trim()) {
      setMatched({ query: "", values: [] });
      setLookupError("");
      return;
    }
    let disposed = false;
    const timer = setTimeout(() => {
      void execute<{ name: string; email: string }[]>("contacts.suggest", {
        query: draft.trim(),
        limit: 12,
      })
        .then((list) => {
          if (!disposed) {
            setMatched({ query: draft.trim().toLowerCase(), values: list });
            setLookupError("");
          }
        })
        .catch((error) => {
          if (!disposed) setLookupError(String(error));
        });
    }, 60);
    return () => {
      disposed = true;
      clearTimeout(timer);
    };
  }, [draft]);
  const existing = new Set(tokens.map((v) => recipientEmail(v)?.toLowerCase()));
  const query = draft.trim().toLowerCase();
  const pool = matched.query === query ? matched.values : [];
  const suggestions = query
    ? pool.filter((c) => !existing.has(c.email.toLowerCase())).slice(0, 8)
    : [];
  const shown = open && suggestions.length > 0;
  const publish = (next: string[], text: string) => {
    setTokens(next);
    setDraft(text);
    const serialized = [...next, ...(text ? [text] : [])].join(", ");
    last.current = serialized;
    onChange(serialized);
  };
  const commit = (text = draft) => {
    const next = [...tokens],
      seen = new Set(existing),
      remaining: string[] = [];
    for (const part of splitRecipients(text)) {
      const email = recipientEmail(part);
      if (!email) {
        remaining.push(part);
        continue;
      }
      if (!seen.has(email.toLowerCase())) {
        next.push(part.replace(/^mailto:/i, ""));
        seen.add(email.toLowerCase());
      }
    }
    publish(next, remaining.join(", "));
    setInvalid(remaining.length > 0);
    setOpen(false);
  };
  const choose = (contact: { name: string; email: string }) => {
    const address = contact.name
      ? `${JSON.stringify(contact.name)} <${contact.email}>`
      : contact.email;
    publish([...tokens, address], "");
    setInvalid(false);
    setOpen(false);
    input.current?.focus();
  };
  return (
    <div
      className="recipient-control"
      ref={popupRoot}
      onBlur={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) commit();
      }}
    >
      <div
        className={`recipient-tags ${invalid ? "invalid" : ""}`}
        onClick={() => input.current?.focus()}
      >
        {tokens.map((token, index) => (
          <span
            className="recipient-chip"
            key={`${token}-${index}`}
            title={token}
          >
            <span>{token}</span>
            <button
              type="button"
              disabled={disabled}
              aria-label={`${en ? "Remove" : "移除"} ${recipientEmail(token)}`}
              onClick={(e) => {
                e.stopPropagation();
                publish(
                  tokens.filter((_, i) => i !== index),
                  draft,
                );
              }}
            >
              <X size={11} />
            </button>
          </span>
        ))}
        <input
          ref={input}
          autoFocus={autoFocus}
          disabled={disabled}
          aria-label={label}
          value={draft}
          aria-invalid={invalid || undefined}
          role="combobox"
          aria-autocomplete="list"
          aria-expanded={shown}
          aria-controls={shown ? id : undefined}
          aria-activedescendant={
            shown
              ? `${id}-${Math.min(active, suggestions.length - 1)}`
              : undefined
          }
          onFocus={() => setOpen(true)}
          onChange={(e) => {
            publish(tokens, e.target.value);
            setInvalid(false);
            setOpen(true);
            setActive(0);
          }}
          onPaste={(e) => {
            const text = e.clipboardData.getData("text/plain");
            if (/[,;\n\r，；]/.test(text)) {
              e.preventDefault();
              const node = e.currentTarget;
              commit(
                draft.slice(0, node.selectionStart ?? draft.length) +
                  text +
                  draft.slice(node.selectionEnd ?? draft.length),
              );
            }
          }}
          onKeyDown={(e) => {
            if (e.nativeEvent.isComposing) return;
            if (e.key === "ArrowDown" && shown) {
              e.preventDefault();
              setActive((v) => (v + 1) % suggestions.length);
            } else if (e.key === "ArrowUp" && shown) {
              e.preventDefault();
              setActive(
                (v) => (v + suggestions.length - 1) % suggestions.length,
              );
            } else if (e.key === "Escape") {
              if (open) e.stopPropagation();
              setOpen(false);
            } else if (e.key === "Enter") {
              e.preventDefault();
              if (shown && !recipientEmail(draft))
                choose(suggestions[Math.min(active, suggestions.length - 1)]);
              else commit();
            } else if (
              [",", ";", "，", "；"].includes(e.key) &&
              recipientEmail(draft)
            ) {
              e.preventDefault();
              commit();
            } else if (e.key === "Tab" && draft.trim()) commit();
            else if (e.key === "Backspace" && !draft && tokens.length) {
              e.preventDefault();
              publish(tokens.slice(0, -1), tokens[tokens.length - 1]);
            }
          }}
        />
      </div>
      {lookupError && (
        <small className="recipient-error">
          {en ? "Contact matching unavailable" : "联系人匹配暂不可用"}
        </small>
      )}
      {invalid && (
        <small className="recipient-error">
          {en ? "Check the email address" : "请检查邮箱地址"}
        </small>
      )}
      {shown && (
        <div
          className="recipient-suggestions"
          role="listbox"
          id={id}
          aria-label={en ? "Matching contacts" : "匹配联系人"}
        >
          {suggestions.map((contact, index) => (
            <button
              type="button"
              role="option"
              id={`${id}-${index}`}
              aria-selected={index === active}
              key={contact.email}
              onMouseDown={(e) => e.preventDefault()}
              onMouseEnter={() => setActive(index)}
              onClick={() => choose(contact)}
            >
              <strong>{contact.name || contact.email}</strong>
              <small>{contact.email}</small>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
