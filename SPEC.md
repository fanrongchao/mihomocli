# 项目规格说明（给 codex / AI 生成器用）
# 目标：用 Rust 实现一个 mihomo/clash 订阅管理的 TUI 工具，支持“多个订阅 + 模板”合并成最终配置，能写到本地文件，结构清晰，便于扩展到 external-controller。

############################################################
# 1. 项目信息
############################################################
project_name: "mihomo-cli / mihomocli"
language: "Rust"
rust_edition: "2021"
description: >
  一个纯本地运行的 TUI 工具，用来管理和合并多个 mihomo/clash 兼容的订阅文件。
  参考 clash-verge-rev 的“订阅 + merge + 模板”思路，但不做桌面 GUI，仅做 TUI。
  用户可以添加多个订阅，选择一个模板，然后合并生成最终的 config.yaml，保存到本地，未来可推送到 mihomo/clash 的 external-controller。
  支持附加 “base-config”（例如 clash-verge-rev 导出的 clash-verge.yaml），在 CLI 合并时继承端口/DNS/规则/代理分组等元数据，保证输出与 upstream 一致。

############################################################
# 2. 技术栈要求
############################################################
runtime: "tokio"
http_client: "reqwest"
yaml: "serde_yaml"
tui: "ratatui + crossterm"   # 不用 GUI
config_dir_helper: "directories"
logging: "tracing" or "tracing-subscriber"
error_handling: "anyhow" or "thiserror"
test: "cargo test" with unit tests on merge logic
dev_shell: "nix flake develop (nix develop)"

############################################################
# 附加：运行时校验（mihomo -t）
############################################################
runtime_validate: |
  提供 `mihomo-cli test` 子命令，调用本机 `mihomo` 二进制执行 `-t` 校验：
    - 默认参数：`mihomo -d ~/.config/mihomocli -f ~/.config/mihomocli/output/config.yaml -m -t`
    - 可通过 `--mihomo-bin`、`--mihomo-dir`、`--config` 覆盖。

############################################################
# 附加：Dev Rules 的 via 回退策略
############################################################
dev_rules_via_fallback: |
  当 `--dev-rules` 开启且目标组名（默认 `Proxy` 或 `--dev-rules-via` 指定）在合并结果中不存在时：
    1) 优先选择存在的 `🚀 节点选择` 组；
    2) 否则选择第一个分组名；
    3) 否则选择第一个代理名；
    4) 都不存在时使用 `DIRECT`；
  若发生回退，会输出 warn 日志，提示实际使用的 via。

############################################################
# 3. 工作区结构（Rust workspace）
############################################################
# 要求生成器建立 workspace，拆成 core + 前端 两个 crate。
# 本仓库前端实现为 CLI（`crates/cli`），TUI 可作为后续扩展。
workspace_layout:
  root:
    - Cargo.toml (workspace)
    - crates/core/Cargo.toml
    - crates/tui/Cargo.toml
    - examples/ (可选，放模板示例)
  crates/core:
    src:
      - lib.rs
      - model.rs          # clash/mihomo 配置模型
      - subscription.rs   # 订阅源模型 + 拉取 + 缓存
      - template.rs       # 模板加载
      - merge.rs          # 合并逻辑（模板 + 多订阅）
      - output.rs         # 写出 / 部署接口
      - storage.rs        # 保存订阅列表到本地
  crates/cli:
    src:
      - main.rs   # clap + orchestration

############################################################
# 4. 配置路径约定（重要）
############################################################
# 所有路径要支持 Linux / NixOS，默认走用户目录
config_paths:
  app_config: "~/.config/mihomocli/app.yaml"
  subscriptions: "~/.config/mihomocli/subscriptions.yaml"
  templates_dir: "~/.config/mihomocli/templates/"
  cache_dir: "~/.cache/mihomocli/subscriptions/"
  output_path: "~/.config/mihomocli/output/config.yaml"
# 如果目录不存在要自动创建

############################################################
# 5. 核心业务概念
############################################################
# 5.1 订阅（Subscription）
rust_struct_subscription: |
  use chrono::{DateTime, Utc};
  use std::path::PathBuf;

  #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
  pub struct Subscription {
      pub id: String,                  // uuid 或手动生成
      pub name: String,                // 在 TUI 中显示的名字
      #[serde(default)]
      pub url: Option<String>,         // 远程订阅
      #[serde(default)]
      pub path: Option<PathBuf>,       // 本地订阅文件
      #[serde(default)]
      pub last_updated: Option<DateTime<Utc>>,
      #[serde(default)]
      pub etag: Option<String>,
      #[serde(default)]
      pub last_modified: Option<String>,
      #[serde(default)]
      pub kind: SubscriptionKind,
      #[serde(default = "default_true")]
      pub enabled: bool,
  }

  #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
  #[serde(rename_all = "lowercase")]
  pub enum SubscriptionKind {
      Clash,     // 普通 clash/mihomo yaml
      Merge,     // 类似 clash-verge-rev 的 merge 类型，后续拓展
      Script,    // 预留
  }

  fn default_true() -> bool { true }

subscription_storage_format: |
  # ~/.config/mihomocli/subscriptions.yaml
  current: "main"
  items:
    - id: "main"
      name: "主订阅"
      url: "https://example.com/sub.yaml"
      kind: "clash"
      enabled: true
    - id: "local"
      name: "本地订阅"
      path: "/home/user/sub-local.yaml"
      kind: "clash"
      enabled: false

# 5.2 模板（Template）
template_rules: |
  - 模板就是一个本地 YAML，作为“骨架配置”
  - 放在 ~/.config/mihomocli/templates/ 下
  - 允许多个模板，TUI 里可切换当前模板
  - 模板负责全局参数：端口、mode、allow-lan、log-level、external-controller、proxy-groups 框架
  - 订阅负责提供：proxies、proxy-groups(补充)、rules(补充)
  - 后续可引入简单变量替换（比如 {{secret}}），先预留接口，不必一次性实现
  - 当用户指定 base-config 时，模板提供的结构在合并节点后再被 base-config 覆盖（端口、DNS、rules、proxy-groups 等），行为对齐 clash-verge-rev

template_example: |
  # ~/.config/mihomocli/templates/default.yaml
  port: 7890
  socks-port: 7891
  redir-port: 7892
  allow-lan: true
  mode: Rule
  log-level: info
  external-controller: "0.0.0.0:9090"
  secret: ""
  proxy-groups:
    - name: "🚀 节点选择"
      type: select
      proxies: []
  rules:
    - MATCH,🚀 节点选择

cli_overrides_external_controller: |
  # 通过 CLI 覆盖 external-controller 主机/端口与 secret：
  # 优先级：CLI 标志 > base-config > 模板 > 订阅
  #
  # 示例：
  #   mihomo-cli merge \
  #     --template examples/cvr_template.yaml \
  #     -s examples/subscription.yaml \
  #     --external-controller-url 0.0.0.0 \
  #     --external-controller-port 19090 \
  #     --external-controller-secret testsecret
  #
  # 效果：输出配置包含
  #   external-controller: 0.0.0.0:19090
  #   secret: testsecret
  #
  # 如果仅提供其中之一：
  # - 仅提供端口时，主机名沿用已存在配置（模板/base-config 中的 external-controller），默认为 127.0.0.1；
  # - 仅提供主机时，端口沿用已存在配置，默认 9090；
  # - 仅提供 secret 时，仅更新 secret 字段。

# 5.3 Clash/Mihomo 配置模型（简化版）
rust_struct_clash_config: |
  use serde::{Serialize, Deserialize};
  use std::collections::BTreeMap;

  #[derive(Debug, Clone, Serialize, Deserialize, Default)]
  pub struct ClashConfig {
      #[serde(default)]
      pub port: Option<u16>,
      #[serde(rename = "socks-port")]
      pub socks_port: Option<u16>,
      #[serde(rename = "redir-port")]
      pub redir_port: Option<u16>,

      #[serde(default)]
      pub proxies: Vec<serde_yaml::Value>,

      #[serde(rename = "proxy-groups", default)]
      pub proxy_groups: Vec<serde_yaml::Value>,

      #[serde(default)]
      pub rules: Vec<String>,

      // 其他字段收敛到 extra，便于向前兼容
      #[serde(flatten)]
      pub extra: BTreeMap<String, serde_yaml::Value>,
  }

# 这样能兼容用户提供的类似：
# proxies:
#   - { name: "剩余流量：967.25 GB", type: trojan, ... }

############################################################
# 6. 合并逻辑（核心）
############################################################
merge_goal: >
  把 “一个模板” + “多个启用的订阅” 合成一个最终的 ClashConfig，
  合并时以模板为主，订阅只追加（proxies / rules），proxy-groups 要做按名字的合并或追加。
  如果提供 base-config，还要在节点合并后复用 base-config 的端口、DNS、规则、代理分组等信息。
merge_rules_detailed: |
  1. 读取模板 YAML -> ClashConfig (base)
  2. 读取所有 enabled=true 的订阅：
     - 如果是远程：用 reqwest 拉取，带上 If-None-Match / If-Modified-Since
     - 如果 304 / 网络失败：用本地缓存
     - 解析成 ClashConfig
  3. 最终合并策略：
     - 标量字段 (port/socks-port/redir-port/mode/log-level/external-controller/secret)：
       以模板为准，订阅不要覆盖
     - proxies:
  out.proxies.extend(sub.proxies)

############################################################
# 7. 订阅抓取与解析（UA 与 base64 开关）
############################################################
- CLI 默认使用 `clash-verge/v2.4.2` 作为 User-Agent 抓取订阅，许多服务端会因此返回带 rules 的 Clash YAML（包含大量 DOMAIN-SUFFIX）。
- CLI 启动时会确保 `~/.config/mihomocli/templates/cvr_template.yaml` 存在，并在缺失时写入嵌入的默认模板（同 examples/cvr_template.yaml）。
- 解析策略：
  - 优先尝试将响应作为 Clash YAML 解析；
  - `--subscription-allow-base64` 未开启时，不再尝试 base64 解码订阅；
  - 显式开启 `--subscription-allow-base64` 时，允许解析 base64/分享链接清单（trojan/vmess/ss）。
- 可通过 `--subscription-ua` 覆盖默认 UA。
- `--dev-rules` 默认开启，会在最终输出前插入一批常用开发依赖域名（涵盖 GitHub/GitLab、Go module proxy、npm/yarn/pnpm、PyPI、crates.io、Kubernetes/k3s 镜像与下载源、Docker/GCR、cache.nixos.org，以及主流 AI 编程代理如 OpenAI/Codex、Anthropic Claude、Google Gemini、Cursor、OpenCode 等）的 proxy 规则。默认指向 `Proxy`，可用 `--dev-rules-via` 覆盖；需要查看默认列表时可使用 `--dev-rules-show`，若不需要可通过 `--no-dev-rules` 禁用。
- Fake‑IP 相关：
  - `--fake-ip-bypass <PATTERN>`：将 PATTERN 追加到 `dns.fake-ip-filter`，并确保 `fake-ip-filter-mode: blacklist`。用于“指定的域名不走 fake‑ip”与规避 DNS 劫持。
  - `--fake-ip-filter-add <PATTERN>`：旧标志，仅追加到 `dns.fake-ip-filter`，不改变 mode。
  - `--fake-ip-filter-mode <blacklist|whitelist>`：显式设置假 IP 过滤模式（高级用法）。
  - 推荐使用 `--fake-ip-bypass` 来添加豁免：例如 `--fake-ip-bypass '+.zhsjf.cn' --fake-ip-bypass 'hs.zhsjf.cn'`。
- 示例提供商（用于本地端到端验证）：
  `https://example.com/sub.yaml`
     - rules:
       out.rules.extend(sub.rules)
     - proxy-groups:
       需要一个 `merge_proxy_groups(template_groups, sub_groups)`：
         - 以 name 为 key
         - 如果模板已经有这个 group，就尝试把订阅里的 proxies 名字 append 进去
         - 如果订阅有新 group，模板没有，就追加到结果末尾
     - extra:
       对于 sub.extra 中的 key，如果模板里没有，就插入；有就保持模板
  4. 合并完成后，把所有 proxies 的名字收集起来，回填到默认的“🚀 节点选择”里（如果存在）
  5. 如果用户提供 base-config：
       - 端口 / socks-port / redir-port / tun / profile 等键以 base-config 为准（覆盖合并结果）
       - rules 直接替换为 base-config 的 rules
       - proxy-groups 结构沿用 base-config，proxies 列表用合并后的节点名称重建
       - base-config 中的 dns/hosts/flatten key (extra) 覆盖或补齐输出

merge_rust_skeleton: |
  pub fn merge(template: ClashConfig, subs: Vec<ClashConfig>) -> ClashConfig {
      let mut out = template;

      for sub in subs {
          // proxies
          out.proxies.extend(sub.proxies);

          // rules
          out.rules.extend(sub.rules);

          // proxy-groups
          out.proxy_groups = merge_proxy_groups(out.proxy_groups, sub.proxy_groups);

          // extra
          for (k, v) in sub.extra {
              out.extra.entry(k).or_insert(v);
          }
      }

      out
  }

  fn merge_proxy_groups(
      mut base: Vec<serde_yaml::Value>,
      incoming: Vec<serde_yaml::Value>,
  ) -> Vec<serde_yaml::Value> {
      // 这里让生成器实现：按 name 找，能合并 proxies 字段
      // 如果找不到同名就 push
      base
  }

merge_tests_to_generate: |
  - test_merge_ports_template_wins
    模板有 port=7890，订阅有 port=8888，合并后仍然是 7890
  - test_merge_proxies_append
    两个订阅各有1个proxy，合并后是2个
  - test_merge_proxy_groups_by_name
    模板有 "🚀 节点选择"，订阅也带了 proxies，合并后这个 group 里能看到订阅的代理名
  - test_merge_rules_append
    模板 rules 在前，订阅 rules 在后，顺序保持

############################################################
# 7. 订阅拉取与缓存
############################################################
fetch_requirements: |
  - 使用 reqwest 异步拉取
  - 如果订阅有 etag/last-modified，下次请求带上
  - 如果返回 304 或网络失败，就读 ~/.cache/mihomocli/subscriptions/{id}.yaml
  - 拉取成功后要写入缓存
  - 订阅统一解析成 ClashConfig，解析失败要在 TUI 显示失败
  - 支持 http/https，暂不必支持 socks（预留）

############################################################
# 8. 输出/部署接口
############################################################
# 先做本地文件落地，将来可以扩展到 external-controller
rust_output_trait: |
  #[async_trait::async_trait]
  pub trait ConfigDeployer {
      async fn deploy(&self, yaml: &str) -> anyhow::Result<()>;
  }

  pub struct FileDeployer {
      pub path: std::path::PathBuf,
  }

  #[async_trait::async_trait]
  impl ConfigDeployer for FileDeployer {
      async fn deploy(&self, yaml: &str) -> anyhow::Result<()> {
          tokio::fs::create_dir_all(self.path.parent().unwrap()).await?;
          tokio::fs::write(&self.path, yaml).await?;
          Ok(())
      }
  }

  // 预留：对接 mihomo/clash external-controller 的 HTTP 实现
  pub struct HttpDeployer {
      pub endpoint: String,        // e.g. http://127.0.0.1:9090/configs
      pub secret: Option<String>,
  }

############################################################
# 9. TUI 设计
############################################################
tui_layout: |
  使用 ratatui + crossterm，界面分两栏：
  - 左栏：订阅列表
    - 显示 name, enabled, last_updated, status(ok/fail)
    - 上下键选择，Enter 查看详情
  - 右栏：详情区域（根据当前界面变化）

  界面/页面：
  1) HomeScreen
     - 左：订阅列表
     - 右：当前订阅的摘要（proxies 数量、rules 数量、来源是 URL 还是本地）
     - 键位：
       - r: 刷新当前订阅
       - R: 刷新所有订阅
       - p: 进入“合并预览”界面
       - t: 选择模板
       - q: 退出
  2) SubscriptionDetailScreen
     - 显示当前订阅的原始 YAML（滚动）
     - 显示最近一次拉取时间
  3) MergePreviewScreen
     - 调用 core 的合并函数，得到最终 ClashConfig
     - 转成 YAML 字符串显示（分页/滚动）
     - 按 w 写入到 output_path
     - 按 b 返回

  键位规范：
    - Up/Down: 移动选中订阅
    - Enter: 打开详情
    - q: 返回/退出
    - r: 刷新
    - p: 合并预览
    - w: 写出最终配置

############################################################
# 10. 启动流程
############################################################
startup_flow: |
  1. 读取 app.yaml（如果没有就用默认值并创建）
  2. 读取 subscriptions.yaml（如果没有就创建一个空的）
  3. 读取 templates 目录，加载所有模板，选中 current_template
  4. 构建 AppState { subscriptions, templates, current_template, output_path, last_merge }
  5. 进入 TUI 主循环

############################################################
# 11. 生成顺序（让 codex 按这个来）
############################################################
generation_steps: |
  1. 创建 Cargo workspace，根 Cargo.toml 写好 members = ["crates/core", "crates/tui"]
  2. 先生成 crates/core：
     - model.rs
     - subscription.rs
     - template.rs
     - merge.rs
     - output.rs
     - storage.rs
     全部对外在 lib.rs 里 pub 出去
  3. 在 crates/core 里写最少 3 个单元测试：merge ports、merge proxies、merge proxy groups
  4. 再生成 crates/tui：
     - main.rs 里启动 tokio runtime，初始化 App，进入 TUI
     - app.rs 管整体状态
     - ui.rs 画布局
     - screens/* 各自渲染
  5. 在 tui 里先用 mock 数据跑通界面，再把 core 注进来
  6. 最后在 main.rs 里加命令：按 w 落地到 ~/.config/mihomocli/output/config.yaml

############################################################
# 12. 注意事项
############################################################
notes: |
  - YAML 字段要保留大小写和原有命名（socks-port, proxy-groups）
  - 合并不要破坏用户订阅里带的中文名字（“剩余流量：xxx GB”）
  - 所有 I/O 都要考虑创建目录
  - 错误要能回传到 TUI，用一个简单的 status 字段显示
  - 代码要能在 NixOS 上编译，尽量避免奇怪的系统依赖
  - 不需要 tauri / gtk / electron，只要终端
  - CLI 启动时需要检查 `~/.config/mihomocli/resources/`，若缺失则自动下载 `Country.mmdb` / `geoip.dat` / `geosite.dat`，与 clash-verge-rev 行为保持一致
  - 项目内提供 `resources/base-config.example.yaml` 说明 base-config 结构，实际使用可通过 `--base-config` 指向真实配置文件
  - **未来工作**：如需完全复刻 clash-verge-rev 的最终 YAML 生成流程，需要移植其增强模板（rules/proxy-groups/scripts）和 runtime 配置合并逻辑；当前版本通过引用现有的 base-config 来达成等价输出

############################################################
# 13. 你可以直接丢给 codex 的一句话
############################################################
codex_prompt_stub: |
  请按上面这份规格说明创建一个 Rust workspace，先生成 crates/core 的代码，再生成 crates/tui，
  保证能 cargo build，通过简单的 TUI 列表看到 mock 的订阅列表，
  并且实现模板 + 多订阅的合并函数。
############################################################
# 14. 应用配置与快捷规则（新增）
############################################################
app_config: |
  # ~/.config/mihomocli/app.yaml
  last_subscription_url: "https://example.com/sub.yaml"  # 最近一次成功的订阅 URL（在用户未提供 -s 时可复用）
  custom_rules:                                            # 用户快捷规则，优先级高于订阅/base-config
    - domain: cache.nixos.org
      kind: domain-suffix   # 或 domain / domain-keyword
      via: Proxy            # 代理或分组名称

custom_rules_cli: |
  # CLI 管理命令
  mihomo-cli manage cache show|clear
  mihomo-cli manage custom add --domain <dom> --via <proxy_or_group> [--kind domain|suffix|keyword]
  # 或者添加直连规则（DIRECT）
  mihomo-cli manage custom add --domain <dom> [--kind domain|suffix|keyword] --via direct
  mihomo-cli manage custom list
  mihomo-cli manage custom remove --domain <dom> [--via <proxy_or_group>]
  # 检查域名走向（proxy/direct）
  mihomo-cli manage check --domain <dom>
  # 查看内置 dev 域名列表（默认plain，可选yaml/json）
  mihomo-cli manage dev-list [--format plain|yaml|json]

use_last_flag: |
  # 显式复用最近一次订阅（不默认自动复用，避免误用）
  mihomo-cli merge --template <tpl> --use-last

dev_flow: |
  # 使用 flake 开发环境，并在提交前保持干净：
  nix develop
  cargo fmt
  cargo clippy --all-targets --all-features
  cargo test -p mihomo-core
