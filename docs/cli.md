# mailfabrica-cli

不开窗口用 MailFabrica：管理账户、收信、看信、搜索、发信，还能当 MCP 服务器。出错时信息写 stderr，退出码非零。

```sh
cargo install --path apps/mailfabrica-cli     # 或：cargo run -p mailfabrica-cli -- <子命令>
```

## 全局选项

| 选项 | 作用 |
|---|---|
| `--data DIR` | 数据目录。默认是桌面程序用的目录（`MAILFABRICA_HOME` > 系统默认）。密码放在系统钥匙串（见引擎文档）。 |
| `--demo` | 用内存里的演示邮箱（`demo@mailfabrica.dev`），不碰网络和真实数据。可以放心试每条命令，包括 `send`。 |

全局选项可以写在任何位置。`--json` 是各子命令自己的选项，输出机器可读的 JSON。

## 子命令

| 子命令 | 例子 |
|---|---|
| `accounts [--json]` | `mailfabrica-cli accounts`，列出账户和服务器 |
| `account add <email> …` | `echo "$PW" \| mailfabrica-cli account add me@qq.com --password-stdin`，见下 |
| `account remove <id>` | `mailfabrica-cli account remove me-qq-com`，同时删除密码和本地数据 |
| `sync [account]` | `mailfabrica-cli sync`，收取全部账户；有账户出错时退出码非零 |
| `folders <account> [--json]` | `mailfabrica-cli folders me-qq-com`，文件夹、角色、未读数 |
| `list [--account A] [--folder F] [--limit N] [--unread] [--flat] [--json]` | `mailfabrica-cli list --account me-qq-com --unread --limit 20` |
| `read <key> [--json]` | `mailfabrica-cli read me-qq-com/1234/INBOX`，邮件头 + 正文；本地没有正文时先下载 |
| `search <query> [--limit N] [--json]` | `mailfabrica-cli search 发票` |
| `send …` | 见下 |
| `commands [--search X] [--json]` | `mailfabrica-cli commands --search compose`，全部命令 id 和参数说明 |
| `exec <command> [json]` | `mailfabrica-cli exec message.flag '{"keys":["me-qq-com/1234/INBOX"],"flagged":true}'`，执行任意命令并打印结果 |
| `mcp [--control PORT]` | MCP 服务器，见 [mcp.md](mcp.md) |
| `version` | 打印 `mailfabrica-cli <版本>` |

### `account add`

```sh
mailfabrica-cli account add <email> [--password-stdin | --password P] [--name N] [--login L]
                            [--imap host:port[:tls|starttls|none]] [--smtp host:port[:tls|starttls|none]] [--no-verify]
```

常见邮箱（QQ、163、Gmail、Outlook ……）会自动填服务器；其他的用 `--imap`、`--smtp` 指定。默认先登录验证，`--no-verify` 跳过。
密码用 `--password-stdin` 从 stdin 读一行（不会留在 shell 历史里）；`--password` 方便但会出现在进程列表里。
添加成功后会自动收一次信。QQ、163 等需要“授权码”，不是登录密码。

### `list` 的输出

```text
demo-mailfabrica-dev/12/INBOX  U-R-  2026-10-11 09:30  张三  周报
```

列：邮件键、标记、日期、发件人、主题。标记四位：`U` 未读，`F` 红旗，`R` 已回复，`A` 有附件，没有的位置是 `-`。
不给 `--account` 时列当前视图（全部收件箱）；给了 `--account` 不给 `--folder` 时是该账户的 `INBOX`。

和界面一样，默认按主题归类（偏好 `groupBySubject`，界面里列表右上角的菜单或设置里可以关）：同主题的邮件
（忽略 `Re:`、`Fwd:`、`回复：` 等前缀）合成一行，键、日期、主题取最新一封，发件人列是不同的名字加封数，
标记是整组汇总（有一封未读就是 `U`）。`--limit` 按行算。`--flat` 一封一行。

```text
demo-mailfabrica-dev/15/INBOX  U---  2026-10-11 06:20  星河云安全中心 (4)        账号登录提醒：新设备登录
```

要看组里每一封：`mailfabrica-cli exec view.groups`，或者 `--flat`。`--json` 时归类结果在 `rows` 里
（每行有 `group`、`count`、`unread`、`flagged`、`senders`、`keys` 和最新一封 `message`）。

### `send`

```sh
mailfabrica-cli send --to a@b.com [--to c@d.com] [--cc …] [--bcc …] [--subject S]
                     [--body TEXT | --body-file FILE|-] [--attach PATH]… [--account A]
```

`--to`、`--cc`、`--attach` 可以重复。`--body-file -` 从 stdin 读正文。不给 `--account` 用第一个账户。
发送失败（地址无效、服务器拒绝）时退出码非零并打印原因，不会留下半成品。

## 先在演示邮箱里试

```sh
mailfabrica-cli --demo list
mailfabrica-cli --demo read <list 里的 key>
mailfabrica-cli --demo send --to a@b.com --subject 测试 --body hi
```
