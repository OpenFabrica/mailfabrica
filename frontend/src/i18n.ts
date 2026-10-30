import dictionary from "./translations.json";
let language = "zh-CN";
// setLanguage 在提交会话快照前更新窗口的显示语言。
export function setLanguage(value: unknown) {
  language = value === "en" ? "en" : "zh-CN";
}
// t 只翻译界面固定文案，不翻译邮件和用户数据。
export function t(text: string): string {
  return language === "en"
    ? ((dictionary as Record<string, string>)[text] ?? text)
    : text;
}
