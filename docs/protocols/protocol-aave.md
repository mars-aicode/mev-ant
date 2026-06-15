# Aave

## 协议名称

- 协议名称：Aave
- 当前项目标签：`aave_like`、`aave_v3_l2_pool`
- 当前支持：liquidation

## 合约地址

| 合约 | 地址 | 说明 |
| --- | --- | --- |
| Pool | 动态地址 | 项目从 `trace.to` 识别 |
| L2Pool | 动态地址 | 项目从 `trace.to` 识别 |

## 文档地址

- 项目实现：`docs/project-implementation-guide.md`
- Aave Pool：<https://aave.com/docs/developers/smart-contracts/pool>
- Aave L2Pool：<https://aave.com/docs/developers/smart-contracts/l2-pool>

## ABI 内容

- 完整 ABI JSON：
	- `abi/aave/Pool.json`
	- `abi/aave/IL2Pool.json`
- 来源说明：`Pool.json` 由 Etherscan 已验证主网 Pool 合约导出；`IL2Pool.json` 来自 Aave 官方 `@aave/core-v3` 发布包中的接口 artifact。