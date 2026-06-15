# Ekubo

## 协议名称

- 协议名称：Ekubo
- 当前项目标签：`ekubo`
- 当前支持：swap

## 合约地址

| 合约 | 地址 | 说明 |
| --- | --- | --- |
| Core | `0x00000000000014aA86C5d3c41765bb24e11bd701` | Ethereum mainnet core singleton；项目运行时仍通过 `trace.to` 识别 |
| Pool | 不适用 | 项目使用 `core + pool_id` 表示池子 |

## 文档地址

- 项目实现：`docs/project-implementation-guide.md`
- 项目总览：`README.md`
- Etherscan：<https://etherscan.io/address/0x00000000000014aa86c5d3c41765bb24e11bd701>

## ABI 内容

- 完整 ABI JSON：
	- `abi/ekubo/Core.json`
- 当前项目运行时解码特征：
	- `trace selector`: `0x00000000`
	- `trace input length`: `>= 132 bytes`
	- `log topics`: `[]`
	- `log data length`: `116 bytes`
- 说明：Ekubo core 在 Etherscan 上可以导出完整 ABI，但项目当前的 swap 解码并不是按常规 `topic0 + typed event` 路线做，而是按 zero-selector trace 与 zero-topic log 的实际链上表现恢复 `pool_id` 和 delta。