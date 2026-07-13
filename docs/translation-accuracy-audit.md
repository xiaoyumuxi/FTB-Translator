# StoneBlock 4 Google 网页翻译准确度审计

- 审计日期：2026-07-13
- 测试对象：StoneBlock 4 `1.15.3` 完整 `lang/en_us.snbt`
- 翻译提供商：免 API Key 的 Google 网页翻译
结论：当前结果适合作为机器初译草稿，**不适合直接发布**。

## 覆盖范围与方法

程序化扫描覆盖全部 2,515 个 key、5,789 个文本片段，其中 4,183 个是非空且非纯占位文本。扫描项目包括：

- 原文与译文完全相同
- 中英文残留和异常长度
- 数字、否定词和限定词风险
- 相同原文翻译不一致
- Minecraft 与模组高频术语
- 专有名词误译
- JSON 文本组件能否解析及结构是否保持
- `item → 项目`、`block → 区块`、`tick → 蜱虫` 等常见机器翻译错误

人工复核包括固定随机种子抽样 200 条、全部 44 条 JSON 文本组件、全部 121 条原文未变化片段、全部 30 组同源异译，以及高频术语和严重异常项，合计约 350 个不同片段。

“全量覆盖”表示所有条目均经过规则扫描；准确度比例仍是基于抽样和高风险复核的估算，并非逐条人工定级，合理误差约为 ±5–7 个百分点。

## 准确度估算

| 等级 | 估算比例 | 含义 |
|------|----------|------|
| A | 约 48% | 语义正确，术语和表达基本可直接使用 |
| B | 约 29% | 大意正确，但存在明显机翻腔、术语或一致性问题 |
| C | 约 18% | 关键术语、信息关系或动作对象错误，需要重译 |
| D | 约 5% | 未翻译、内容损坏、严重幻觉或富文本无法解析 |

严格按可直接公开发布的标准，当前准确率估计约为 **45%–55%**。README 中的 99.44% 是现有格式守卫通过率，不是翻译准确率。

## 确认的问题统计

- 44 条 JSON 文本组件中有 19 条目标内容无法解析，占 43.2%。
- 52 个有意义的英文片段完全未译，涉及 26 个 key。
- 其中 14 个 key 是格式守卫主动回退，至少 12 个未被报告标记。
- 相同英文原文出现不同译法，共 30 组。
- `item/items` 被译成“项目”至少 71 处。
- 能量语境中的 `power` 被译成“电源”至少 32 处。
- 普通方块 `block/blocks` 被误译成“区块”至少 27 处。
- `enchanting` 被译成“迷人”至少 11 处。
- `vanilla` 被译成“香草”至少 8 处。
- `Mekanism` 被译成“机制”“机构”“机械”等，并与保留英文混用。
- Echo-Location、Wyvern、Chaotic、Cobble、Mycelial、Eclipse 等术语存在多套译名。

## 代表性问题

| 严重度 | key | 英文 | 当前中文 | 建议译文 |
|--------|-----|------|----------|----------|
| 高 | `chapter.0C812DF8D14584AB.title` | Mekanism | 机制 | 通用机械 / Mekanism |
| 高 | `file.0000000000000001.title` | Stoneblock 4 | 石块 4 | StoneBlock 4 |
| 高 | `chapter.3BF272F9FDD1D8F9.title` | Draconic Evolution | 龙族进化 | 龙之进化 |
| 高 | `chapter.3B99F85218D37371.title` | Useful Items & Tips | 有用的项目&提示 | 实用物品与技巧 |
| 中 | `chapter.391B65CD04A90459.title` | Power | 电源 | 能源 / 电力 |
| 高 | `quest.009FE1E1B3017486.title` | Mob Slaughter Factory | 暴徒屠宰场 | 生物屠宰工厂 |
| 高 | `quest.11C58E38DE9488BE.title` | Mob Mashing | 暴徒捣乱 | 生物碾压 |
| 中 | `quest.007CD364F96C2912.quest_desc` | prevent all fall damage | 防止所有坠落损坏 | 免疫所有摔落伤害 |
| 高 | `quest.00A7DEFCF07BC63A.quest_desc` | Red Katar | 红色拳头 | 红物质拳剑，或采用 ProjectE 官方译名 |
| 高 | `quest.007F4A173D5CA51F.quest_desc` | souls you need for this craft | 这门手艺所需的所有灵魂 | 本次合成所需的全部灵魂 |
| 高 | `quest.00A7C1FE54F86429.quest_desc` | enchanting setup | 迷人设置 | 附魔设施 |
| 高 | `quest.00A7C1FE54F86429.quest_desc` | enchanting table | 迷人桌子 | 附魔台 |
| 高 | `quest.3382AC7FEFE0B76E.title` | Vanilla Netherite Weapons | 香草下界合金武器 | 原版下界合金武器 |
| 高 | `quest.38E2176886386133.quest_desc` | electrum | 金金 | 琥珀金 |
| 高 | `quest.0BEF15E1352A5F72.title` | Matter Replication | 事务复制 | 物质复制 |
| 高 | `task.496455D7AB00E7B8.title` | Constructors | 构造函数 | 构造器 |
| 高 | `quest.77074CE5719763BB.quest_desc` | These charges replenish | 这些费用会补充 | 这些充能次数会逐渐恢复 |
| 高 | `quest.456030ECDFA9CE3B.quest_desc` | Each tick | 每个蜱虫 | 每游戏刻 |
| 高 | `quest.13C8B2286F5184C7.quest_desc` | direction you are looking, up to 100 blocks | 您正在寻找的方向，最远100个街区 | 朝视线方向传送，最远 100 格 |
| 高 | `quest.5BE257CE3FD1FDAC.quest_desc` | upgrade your Infusion setup | 升级您的输液设置 | 升级注入合成设施 |
| 中 | `quest.091F82E2770F3011.title` | Ancient Check 1 | 古代支票 1 | 远古检查 1 |
| 高 | `quest.6BB9FD515DFD8FF1.title` | Applied Energistics 2 | 应用能量学2 | 应用能源 2 |
| 高 | `quest.1069575C4B5BCE39.title` | Refined Storage 2 | 精炼存储2 | 精致存储 2 |
| 高 | `quest.276D2B384F21AC10.title` | Just Dire's Weaponry | 只是可怕的武器 | Just Dire Things 武器 |
| 高 | `quest.7CA687749E953A19.quest_desc` | Right Click with a bullet | 用项目符号右键单击 | 手持法术子弹右键单击 |
| 高 | `quest.66BC4434409F959B.quest_desc` | hurting for Uranium | 伤害铀 | 缺少铀 |
| 高 | `quest.45CB3F325010A5AD.quest_desc` | Experience Obelisk | 体验方尖碑 | 经验方尖碑 |
| 高 | `quest.4C52D3DE2F7701D0.quest_desc` | Crafting the brush | 制作画笔 | 制作刷子 |
| 严重 | `quest.137EC884018376A1.quest_desc` | Ancient Debris rich-text link | 文本顺序被插入 `hoverEvent.action` | 重建 JSON，只翻译展示字段 |
| 严重 | `quest.05CEF45909A11F60.quest_desc` | backpack keybind JSON | JSON 引号变成中文全角引号 | 保持 JSON 语法，只翻译显示文本 |
| 严重 | `quest.4CE414ADD47790B2.quest_desc` | Replicators can be sped up... | 整句保持英文 | 复制器可通过安装复制器外壳来加速…… |
| 高 | `quest.4D754E91D203A89A.quest_desc` | Echo of Guidance | 指导回声 | 指引之回响，并全局统一 |
| 高 | `quest.5D2936B7F8D0F8FA.title` | Block of Ignitium Trade | Ignitium 贸易区块 | Ignitium 块兑换 |
| 高 | `quest.6755CACD4957A4CB.quest_desc` | essential part of your CAD | CAD CAD 的重要组成部分 | CAD 的核心组件 |
| 严重 | `quest.580CDFD2B7675485.quest_desc` | the craft should start | 飞船就会启动 | 合成过程就会启动 |
| 高 | `quest.427682661459BB4F.quest_desc` | either +100% or -100% power | +100% o或-100%力量 | 能量加成会在 +100% 与 -100% 之间变化 |
| 高 | `quest.0089BFB911BD5E5B.quest_desc` | 1 block per scale module / chunk claims | 1个区块 / 块声明 | 每个范围模块增加 1 格 / 区块认领 |
| 高 | `quest.5A04F7A95366D15F.quest_desc` | two blocks away | 两个街区 | 相距两个方块 |
| 高 | `quest.70D5BEA5CC437AB6.quest_desc` | Tome of Scrapping | 刮痧之书 | 拆解之书，或采用模组官方译名 |
| 高 | `task.3A831AEACF765BB4.title` | Quanta Items | 广达项目 | Quanta 物品 / 量子物品，需按 Apotheosis 术语表 |
| 高 | `quest.31C83D9253FFF685.title` | Mekanism Turbine | 机构涡轮机 | 通用机械：涡轮机 |

## 发布判断与改进方向

当前译文不适合直接发布。主要问题不是少数错别字，而是富文本损坏、缺少模组术语库、玩法关键概念错译、同源异译、未译内容，以及部分会直接误导玩家操作的语义错误。

建议按以下顺序改进：

1. 分开展示 API 请求成功率、格式守卫通过率和人工抽检准确率。
2. 建立 Minecraft、模组、物品、方块、机器、Boss、NPC 和整合包叙事术语表。
3. 从模组自带 `zh_cn.json` 提取注册名，优先复用官方本地化。
4. 先解析 JSON 富文本，只翻译允许的 `text` 和展示型 `contents` 字段，再重新序列化和验证。
5. 增加 `item → 物品`、`block → 方块`、`chunk → 区块`、`tick → 游戏刻`、`vanilla → 原版` 等语义规则。
6. 标题和任务说明使用不同的翻译提示与质量标准。
7. 对重复原文使用翻译记忆，禁止同源异译。
8. 增加第二阶段术语修正与语义审校，重点检查否定、数字、方向、单位、快捷键和操作动词。
