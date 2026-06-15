# Curve AMM

## 协议名称

- 协议名称：Curve AMM
- 当前项目标签：`curve`
- 当前支持：swap

## 合约地址

| 合约 | 地址 | 说明 |
| --- | --- | --- |
| Pool | 动态地址 | 项目运行时通过 `log.address` 识别 |

## 文档地址

- 项目实现：`docs/project-implementation-guide.md`
- Curve 文档：<https://docs.curve.finance/>

## ABI 内容

- 完整 ABI JSON：
	- `abi/curve/ThreePool.json`
	- `abi/curve/TriCrypto2.json`
- 来源说明：由 Etherscan 已验证主网代表池导出；当前项目识别 Curve AMM 时以代表性 stable pool / crypto pool ABI 作为完整 JSON 参考。