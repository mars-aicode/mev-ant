# Shiba V2

## 协议名称

- 协议名称：ShibaSwap V2
- 当前项目标签：`shiba_v2`
- 当前支持：swap、mint、burn

## 合约地址

| 合约 | 地址 | 说明 |
| --- | --- | --- |
| Factory | `0xD9CE49caf7299DaF18ffFcB2b84a44fD33412509` | 项目源码 hardcoded factory hint |
| Pool | 动态地址 | 项目运行时通过 `log.address` 识别 |

## 文档地址

- 项目实现：`docs/project-implementation-guide.md`
- 兼容 ABI 参考：<https://developers.uniswap.org/docs/protocols/v3/deployments/v3-ethereum-deployments>

## ABI 内容

- 完整 ABI JSON：
	- `abi/uniswap-v3/IUniswapV3Factory.json`
	- `abi/uniswap-v3/IUniswapV3Pool.json`
- 来源说明：当前项目按 Uniswap V3-compatible 接口识别 Shiba V2 池子，因此直接复用上游完整 JSON ABI。