# Pancake V2

## 协议名称

- 协议名称：PancakeSwap V2
- 当前项目标签：`pancake_v2`
- 当前支持：swap、mint、burn

## 合约地址

| 合约 | 地址 | 说明 |
| --- | --- | --- |
| Factory | `0x1097053Fd2ea711dad45caCcc45EfF7548fCB362` | 项目源码 hardcoded factory hint |
| Pair | 动态地址 | 项目运行时通过 `log.address` 识别 |

## 文档地址

- 项目实现：`docs/project-implementation-guide.md`
- Pancake 文档：<https://docs.pancakeswap.finance/>
- 兼容 ABI 参考：<https://developers.uniswap.org/docs/protocols/v2/deployments>

## ABI 内容

- 完整 ABI JSON：
	- `abi/uniswap-v2/IUniswapV2Factory.json`
	- `abi/uniswap-v2/IUniswapV2Pair.json`
- 来源说明：当前项目按 Uniswap V2-compatible 接口识别 Pancake V2 池子，因此直接复用上游完整 JSON ABI。