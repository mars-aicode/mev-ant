# Uniswap V2

## 协议名称

- 协议名称：Uniswap V2
- 当前项目标签：`uniswap_v2`
- 当前支持：swap、mint、burn

## 合约地址

| 合约 | 地址 | 说明 |
| --- | --- | --- |
| Factory | `0x5C69bEe701ef814a2B6a3EDD4B1652CB9cc5aA6f` | 官方 mainnet factory |
| Router02 | `0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D` | 官方 mainnet router |
| Pair | 动态地址 | 项目运行时通过 `log.address` 识别 |

## 文档地址

- 项目实现：`docs/project-implementation-guide.md`
- 官方部署：<https://developers.uniswap.org/docs/protocols/v2/deployments>

## ABI 内容

- 完整 ABI JSON：
	- `abi/uniswap-v2/IUniswapV2Factory.json`
	- `abi/uniswap-v2/IUniswapV2Router02.json`
	- `abi/uniswap-v2/IUniswapV2Pair.json`
- 来源说明：由 Etherscan 已验证主网合约导出；Pair ABI 使用 `USDC/WETH` 代表池导出，可作为当前项目识别的 Uniswap V2 pair 完整 ABI。