# Sushi

## 协议名称

- 协议名称：SushiSwap
- 当前项目标签：`sushi`
- 当前支持：swap、mint、burn

## 合约地址

| 合约 | 地址 | 说明 |
| --- | --- | --- |
| Factory | `0xc0aeE478e3658e2610c5F7A4A2E1777cE9e4f2Ac` | 项目源码 hardcoded factory hint |
| Pair | 动态地址 | 项目运行时通过 `log.address` 识别 |

## 文档地址

- 项目实现：`docs/project-implementation-guide.md`
- Sushi 文档：<https://docs.sushi.com/>
- 兼容 ABI 参考：<https://developers.uniswap.org/docs/protocols/v2/deployments>

## ABI 内容

- 完整 ABI JSON：
	- `abi/uniswap-v2/IUniswapV2Factory.json`
	- `abi/uniswap-v2/IUniswapV2Pair.json`
- 来源说明：当前项目按 Uniswap V2-compatible 接口识别 Sushi 池子，因此直接复用上游完整 JSON ABI。