import { useEffect, useRef } from "react";

// useDismiss 统一浮层的外部点击、键盘和焦点关闭，并覆盖正文 iframe 内不会冒泡到父文档的操作。
export function useDismiss<T extends HTMLElement = HTMLDivElement>(
  open: boolean,
  onClose: () => void,
) {
  const root = useRef<T | null>(null);
  const close = useRef(onClose);
  close.current = onClose;
  useEffect(() => {
    if (!open) return;
    const outside = (event: Event) => {
      if (root.current && !event.composedPath().includes(root.current))
        close.current();
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.isComposing || event.defaultPrevented)
        return;
      event.stopPropagation();
      close.current();
    };
    // Safari 的按钮点击不一定转移焦点；让下一次进入正文 iframe 能触发父窗口失焦。
    if (root.current && !root.current.contains(document.activeElement))
      root.current
        .querySelector<HTMLElement>("button,input,select,textarea,[tabindex]")
        ?.focus();
    const frameFocus = () => {
      queueMicrotask(() => {
        if (document.activeElement instanceof HTMLIFrameElement)
          close.current();
      });
    };
    window.addEventListener("blur", frameFocus);
    const documents = new Set<Document>();
    // 邮件文档只允许父页面读取；监听指针不向其中注入脚本。
    const listen = (doc: Document) => {
      if (documents.has(doc)) return;
      documents.add(doc);
      doc.addEventListener("pointerdown", outside, true);
      doc.addEventListener("focusin", outside, true);
      doc.addEventListener("keydown", escape, true);
    };
    const frameLoaded = (event: Event) => {
      if (event.target instanceof HTMLIFrameElement) {
        try {
          if (event.target.contentDocument)
            listen(event.target.contentDocument);
        } catch {
          /* 跨来源页面由浏览器隔离，主文档仍处理外部点击。 */
        }
      }
    };
    listen(document);
    document.querySelectorAll("iframe").forEach((frame) => {
      try {
        if (frame.contentDocument) listen(frame.contentDocument);
      } catch {
        /* 无权限的外部文档不读取。 */
      }
    });
    document.addEventListener("load", frameLoaded, true);
    return () => {
      window.removeEventListener("blur", frameFocus);
      document.removeEventListener("load", frameLoaded, true);
      for (const doc of documents) {
        doc.removeEventListener("pointerdown", outside, true);
        doc.removeEventListener("focusin", outside, true);
        doc.removeEventListener("keydown", escape, true);
      }
    };
  }, [open]);
  return root;
}
