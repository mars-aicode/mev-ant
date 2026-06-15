# Pancake V3

## 协议名称

- 协议名称：PancakeSwap V3
- 当前项目标签：`pancake_v3`
- 当前支持：swap、mint、burn

## 合约地址

| 合约 | 地址 | 说明 |
| --- | --- | --- |
| Factory | `0x0BFbCF9fa4f9C56B0F40a671Ad40E0805A091865` | 项目源码 hardcoded factory hint |
| Pool | 动态地址 | 项目运行时通过 `log.address` 识别 |

## 文档地址

- 项目实现：`docs/project-implementation-guide.md`
- Pancake 文档：<https://docs.pancakeswap.finance/>
- 兼容 ABI 参考：<https://developers.uniswap.org/docs/protocols/v3/deployments/v3-ethereum-deployments>

## ABI 内容

- 完整 ABI JSON：
	- `abi/uniswap-v3/IUniswapV3Factory.json`
	- `abi/uniswap-v3/IUniswapV3Pool.json`
- 来源说明：当前项目按 Uniswap V3-compatible 接口识别 Pancake V3 池子，因此先复用上游完整 JSON ABI。