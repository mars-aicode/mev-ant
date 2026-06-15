# Fraxlend

## 协议名称

- 协议名称：Fraxlend
- 当前项目标签：`fraxlend`
- 当前支持：liquidation

## 合约地址

| 合约 | 地址 | 说明 |
| --- | --- | --- |
| Pair | 动态地址 | 项目从 `trace.to` 识别 |

## 文档地址

- 项目实现：`docs/project-implementation-guide.md`
- Frax 文档：<https://docs.frax.finance/>
- 官方仓库：<https://github.com/FraxFinance/fraxlend>

## ABI 内容

- `docs/protocols/abi/fraxlend/FraxlendPair.json`
	- 官方来源：`FraxFinance/fraxlend` 的 `src/liquidatorBot/abis/FraxlendPair.mjs`
	- 已验证包含：`liquidate(uint128,uint256,address)`

Fraxlend Pair 地址按市场动态部署；
