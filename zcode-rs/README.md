# zcode-rs

独立编写的 Rust + GPUI 编码代理，按 [ZCode](https://github.com/zai-org/ZCode) 公开文档里的核心行为重新实现。它不是 Z.ai 官方客户端，也不包含上游源代码。

## 构建

需要 Rust 1.85 或更新（依赖的 `gpui` 0.2.2 使用 Rust 2024 edition）。Linux 上还需要 X11 或 Wayland、Vulkan、fontconfig 和 xkbcommon 的开发库。

`gpui` 0.2.2 依赖的 `xattr` 0.2.3 在当前 libc 上缺少 `ENOATTR`。`vendor/xattr` 是这份 crate 的本地补丁，Linux 上改用 `ENODATA`。许可证仍是它原来的 MIT/Apache-2.0。

```bash
cd zcode-rs
cargo test --no-default-features --lib
cargo build
```

发布构建：

```bash
cargo build --release
./target/release/zcode-rs
```

调试构建的可执行文件是 `target/debug/zcode-rs`。`--version` 只打印版本，不打开窗口。

没有 GPU 时，可以用 lavapipe 软件渲染，例如：

```bash
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json ./target/debug/zcode-rs
```

需要可用的 `DISPLAY` 或 `WAYLAND_DISPLAY`。只设置了 `DISPLAY` 时走 X11。

## 运行

```bash
export ZCODE_API_BASE=https://api.openai.com/v1
export ZCODE_API_KEY=你的密钥
export ZCODE_MODEL=gpt-4o-mini
export ZCODE_WORKSPACE="$PWD"
./target/debug/zcode-rs
```

密钥也可以放在 `OPENAI_API_KEY`。`ZCODE_API_KEY` 优先。不要把密钥写进源码或会话文件。

| 变量 | 作用 |
| --- | --- |
| `ZCODE_API_BASE` | OpenAI 兼容 API 根地址，默认 `https://api.openai.com/v1`。若已经以 `/chat/completions` 结尾则不再拼接。 |
| `ZCODE_API_KEY` | 发送给该端点的 Bearer 令牌。 |
| `OPENAI_API_KEY` | 没有 `ZCODE_API_KEY` 时的备用令牌。 |
| `ZCODE_MODEL` | 模型名，默认 `gpt-4o-mini`。 |
| `ZCODE_WORKSPACE` | 新会话的工作区，默认是启动时的当前目录。工具只能在这个目录里读写文件。 |
| `ZCODE_DATA_BASE_DIR` | 设置后，会话写到 `$ZCODE_DATA_BASE_DIR/.zcode-rs/sessions/`。 |
| `ZCODE_MAX_STEPS` | 单轮用户消息里最多向模型往返多少次，默认 16，上限 32。 |

未设置 `ZCODE_DATA_BASE_DIR` 时，会话写到 `~/.zcode-rs/sessions/*.json`。这和官方 ZCode 的 `~/.zcode` 数据目录分开。

窗口左侧是会话列表，中间是对话，底部是输入框。Enter 发送，Shift+Enter 换行，Ctrl/Cmd+V 粘贴。发送后模型可以调用四个内置工具：`read_file`、`list_dir`、`edit_file`、`run_command`。

`run_command` 的工作目录是会话工作区，并会从子进程环境里去掉常见的密钥变量。它不是操作系统沙箱，命令仍然可能影响工作区以外的系统。

## 扩展点

`Extension` 可以注册工具。内置工具在 `builtin` 扩展里。后注册的同名工具会替换先前的实现。这一轮没有插件市场、MCP、技能或 Hook。

## 许可

新代码使用 Apache-2.0，见 `LICENSE` 和 `NOTICE`。
