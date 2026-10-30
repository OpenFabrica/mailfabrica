import { t } from "./i18n";
import type { Params, Snapshot } from "./types";
import { snapshot } from "./api";
const service = "github.com/openfabrica/mailfabrica/internal/desktop.Service";
// rectangle 输出 UI 检查所需的客户端坐标，不修改指针或窗口焦点。
function rectangle(node: Element | null) {
  if (!node) return null;
  const r = node.getBoundingClientRect();
  return { x: r.x, y: r.y, width: r.width, height: r.height };
}
// boundedNumber 为坐标与字符偏移实施有界输入检查。
function boundedNumber(p: Params, k: string, max: number) {
  const n = p[k];
  if (typeof n !== "number" || !Number.isFinite(n) || n < 0 || n > max)
    throw new Error(`${k} 数值无效`);
  return n;
}
// readerDocument 返回无脚本权限的正文文档，仅宿主固定操作可以读取。
function readerDocument() {
  const frame = document.querySelector<HTMLIFrameElement>(
    ".reader-content iframe",
  );
  if (!frame?.contentDocument) throw new Error(t("邮件正文尚未加载"));
  return { frame, doc: frame.contentDocument };
}
// performUI 实现固定控制方法，不向外部调用方提供脚本执行能力。
export async function performUI(method: string, p: Params) {
  switch (method) {
    case "ui.inspect": {
      const state = await snapshot();
      return {
        ...state,
        protocolVersion: 2,
        backend: "wails-webview",
        capabilities: {
          screenshot: "dom-without-email-body",
          syntheticInput: true,
          reader: true,
        },
        panes: {
          sidebar: rectangle(document.querySelector(".sidebar")),
          list: rectangle(document.querySelector(".mail-list")),
          reader: rectangle(document.querySelector(".reader")),
          calendar: rectangle(document.querySelector(".calendar-main")),
        },
        dialog:
          document.querySelector("[role=dialog]")?.getAttribute("aria-label") ||
          null,
        visibility: { visible: !document.hidden },
        reader: {
          backend: "webview-iframe",
          status: state.message?.state || "missing",
          remoteImages: state.prefs.remoteImages,
          rect: rectangle(document.querySelector(".reader-content iframe")),
        },
      };
    }
    case "ui.screenshot": {
      const { toPng } = await import("html-to-image");
      const data = await toPng(document.documentElement, {
        pixelRatio: 1,
        skipFonts: true,
        filter: (node) => !(node instanceof HTMLIFrameElement),
      });
      return {
        base64: data.split(",")[1],
        backend: "dom-without-email-body",
        includesEmailBody: false,
      };
    }
    case "ui.click": {
      let node: Element | null = null;
      if (typeof p.target === "string") {
        node =
          [...document.querySelectorAll("[data-testid]")].find(
            (n) => n.getAttribute("data-testid") === p.target,
          ) || null;
      } else {
        node = document.elementFromPoint(
          boundedNumber(p, "x", 20000),
          boundedNumber(p, "y", 20000),
        );
      }
      if (!(node instanceof HTMLElement)) throw new Error(t("点击目标不存在"));
      node.focus();
      node.click();
      if (p.count === 2)
        node.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
      return { queued: true };
    }
    case "ui.text": {
      if (typeof p.text !== "string" || p.text.length > 1e6)
        throw new Error(t("text 无效"));
      const node = document.activeElement;
      if (
        node instanceof HTMLInputElement ||
        node instanceof HTMLTextAreaElement
      ) {
        const proto =
          node instanceof HTMLTextAreaElement
            ? HTMLTextAreaElement.prototype
            : HTMLInputElement.prototype;
        Object.getOwnPropertyDescriptor(proto, "value")?.set?.call(
          node,
          p.text,
        );
        node.dispatchEvent(new Event("input", { bubbles: true }));
        node.dispatchEvent(new Event("change", { bubbles: true }));
      } else if (node instanceof HTMLElement && node.isContentEditable) {
        document.execCommand("insertText", false, p.text);
      } else throw new Error(t("没有聚焦的编辑区域"));
      return { queued: true };
    }
    case "ui.key": {
      if (typeof p.key !== "string") throw new Error(t("key 无效"));
      const aliases: Record<string, string> = {
        Down: "ArrowDown",
        Up: "ArrowUp",
        Enter: "Enter",
        Return: "Enter",
        Esc: "Escape",
        Delete: "Delete",
        Backspace: "Backspace",
      };
      const key = aliases[p.key] || p.key;
      const target = document.activeElement || document.body;
      target.dispatchEvent(
        new KeyboardEvent("keydown", {
          key,
          bubbles: true,
          metaKey: Boolean(p.cmd),
          ctrlKey: Boolean(p.ctrl),
          altKey: Boolean(p.alt),
          shiftKey: Boolean(p.shift),
        }),
      );
      target.dispatchEvent(new KeyboardEvent("keyup", { key, bubbles: true }));
      return { queued: true };
    }
    case "ui.calendar":
      document.dispatchEvent(
        new CustomEvent("mailfabrica:calendar", { detail: p }),
      );
      return { queued: true };
    case "ui.readerMode":
      if (p.mode !== "plain" && p.mode !== "html")
        throw new Error(t("mode 无效"));
      document.dispatchEvent(
        new CustomEvent("mailfabrica:readerMode", { detail: p.mode }),
      );
      return { queued: true };
    case "ui.readerReload": {
      const { frame } = readerDocument();
      frame.srcdoc = frame.srcdoc;
      return { queued: true };
    }
    case "ui.readerActivateLink": {
      if (typeof p.text !== "string" || !p.text.trim())
        throw new Error(t("链接文字无效"));
      const { frame, doc } = readerDocument();
      const links = [
        ...doc.querySelectorAll<HTMLAnchorElement>("a[href]"),
      ].filter((node) => node.textContent?.trim() === p.text);
      if (links.length !== 1) throw new Error(t("未找到唯一匹配的邮件链接"));
      links[0].scrollIntoView({ block: "center", inline: "nearest" });
      await new Promise<void>((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
      );
      const nativeLink = [
        ...(frame.parentElement?.querySelectorAll<HTMLAnchorElement>(
          "[data-email-link]",
        ) || []),
      ].find(
        (node) =>
          node.getAttribute("aria-label") === p.text &&
          node.getAttribute("href") === links[0].getAttribute("href"),
      );
      if (nativeLink) {
        const bounds = nativeLink.getBoundingClientRect();
        nativeLink.dispatchEvent(
          new MouseEvent("click", {
            bubbles: true,
            cancelable: true,
            detail: 1,
            clientX: bounds.left + bounds.width / 2,
            clientY: bounds.top + bounds.height / 2,
          }),
        );
      } else links[0].click();
      return { queued: true };
    }
    case "ui.readerScroll": {
      const { doc } = readerDocument();
      doc.documentElement.scrollTop = boundedNumber(p, "top", 200000);
      return { top: doc.documentElement.scrollTop };
    }
    case "ui.readerSelect": {
      const { doc } = readerDocument();
      const start = boundedNumber(p, "start", 1000000),
        end = boundedNumber(p, "end", 1000000);
      if (end < start) throw new Error(t("end 不能小于 start"));
      const walker = doc.createTreeWalker(doc.body, NodeFilter.SHOW_TEXT, {
        acceptNode: (n) =>
          n.parentElement?.closest("style,script")
            ? NodeFilter.FILTER_REJECT
            : NodeFilter.FILTER_ACCEPT,
      });
      let offset = 0;
      let startNode: Node | null = null,
        endNode: Node | null = null,
        startOffset = 0,
        endOffset = 0;
      while (walker.nextNode()) {
        const node = walker.currentNode;
        const chars = Array.from(node.textContent || "");
        if (startNode === null && start <= offset + chars.length) {
          startNode = node;
          startOffset = chars
            .slice(0, Math.max(0, start - offset))
            .join("").length;
        }
        if (end <= offset + chars.length) {
          endNode = node;
          endOffset = chars.slice(0, Math.max(0, end - offset)).join("").length;
          break;
        }
        offset += chars.length;
      }
      if (!startNode || !endNode) throw new Error(t("选区超出正文范围"));
      const range = doc.createRange();
      range.setStart(startNode, startOffset);
      range.setEnd(endNode, endOffset);
      const selection = doc.getSelection();
      selection?.removeAllRanges();
      selection?.addRange(range);
      return { text: selection?.toString() || "" };
    }
    case "ui.readerCopy": {
      const { doc } = readerDocument();
      const { Call } = await import("@wailsio/runtime");
      await Call.ByName(
        `${service}.CopyText`,
        doc.getSelection()?.toString() || "",
      );
      return { copied: true };
    }
    case "ui.readerHitTest": {
      const { doc } = readerDocument();
      const node = doc.elementFromPoint(
        boundedNumber(p, "x", 20000),
        boundedNumber(p, "y", 20000),
      );
      return {
        hitTest: {
          cursor: node?.closest("a")
            ? "hand"
            : node?.textContent
              ? "text"
              : "arrow",
        },
      };
    }
    case "ui.drag":
      throw new Error(
        t("Wails 控制协议 v2 请使用 ui.readerSelect 或真实指针完成正文选择"),
      );
    default:
      throw new Error(t("未知 UI 方法"));
  }
}
// attachControl 将宿主异步请求连接到固定 UI 适配，销毁时取消监听。
export async function attachControl() {
  const { Events, Call } = await import("@wailsio/runtime");
  return Events.On("mailfabrica:control", async (event) => {
    const data = event.data as {
      id: string;
      method: string;
      params: Params;
      window?: string;
    };
    const compose = new URLSearchParams(location.search).get("compose");
    const currentWindow = compose ? `compose-${compose}` : "main";
    if ((data.window || "main") !== currentWindow) return;
    let value: unknown = null,
      error = "";
    try {
      value = await performUI(data.method, data.params || {});
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
    await Call.ByName(
      `${service}.UIResult`,
      data.id,
      JSON.stringify(value),
      error,
    );
  });
}
