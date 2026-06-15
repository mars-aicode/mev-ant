# Uniswap V3

## 协议名称

- 协议名称：Uniswap V3
- 当前项目标签：`uniswap_v3`
- 当前支持：swap、mint、burn

## 合约地址

| 合约 | 地址 | 说明 |
| --- | --- | --- |
| Factory | `0x1F98431c8aD98523631AE4a59f267346ea31F984` | 官方 mainnet factory |
| SwapRouter | `0xE592427A0AEce92De3Edee1F18E0157C05861564` | 官方 mainnet router |
| SwapRouter02 | `0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45` | 官方 mainnet router |
| NonfungiblePositionManager | `0xC36442b4a4522E871399CD717aBDD847Ab11FE88` | 官方 mainnet position manager |
| Pool | 动态地址 | 项目运行时通过 `log.address` 识别 |

## 文档地址

- 项目实现：`docs/project-implementation-guide.md`
- 官方部署：<https://developers.uniswap.org/docs/protocols/v3/deployments/v3-ethereum-deployments>

## ABI 内容

- 完整 ABI JSON：
	- `abi/uniswap-v3/IUniswapV3Factory.json`
	- `abi/uniswap-v3/ISwapRouter.json`
	- `abi/uniswap-v3/SwapRouter02.json`
	- `abi/uniswap-v3/NonfungiblePositionManager.json`
	- `abi/uniswap-v3/IUniswapV3Pool.json`
- 来源说明：由 Etherscan 已验证主网合约导出；Pool ABI 使用 `WETH/USDC 0.05%` 代表池导出，可作为当前项目识别的 Uniswap V3 pool 完整 ABI。