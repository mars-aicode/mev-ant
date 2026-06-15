# Uniswap V4

## 协议名称

- 协议名称：Uniswap V4
- 当前项目标签：`uniswap_v4`
- 当前支持：swap、modifyLiquidity

## 合约地址

| 合约 | 地址 | 说明 |
| --- | --- | --- |
| PoolManager | `0x000000000004444c5dc75cB358380D2e3dE08A90` | 官方 mainnet singleton |
| PositionManager | `0xbd216513d74c8cf14cf4747e6aaa6420ff64ee9e` | 官方 mainnet periphery |
| StateView | `0x7ffe42c4a5deea5b0fec41c94c136cf115597227` | 官方 mainnet view helper |
| Quoter | `0x52f0e24d1c21c8a0cb1e5a5dd6198556bd9e1203` | 官方 mainnet quoter |
| Universal Router | `0x66a9893cc07d91d95644aedd05d03f95e1dba8af` | 官方 mainnet router |

## 文档地址

- 项目实现：`docs/project-implementation-guide.md`
- 官方部署：<https://developers.uniswap.org/docs/protocols/v4/deployments>

## ABI 内容

- 完整 ABI JSON：
	- `abi/uniswap-v4/PoolManager.json`
	- `abi/uniswap-v4/PositionManager.json`
	- `abi/uniswap-v4/StateView.json`
	- `abi/uniswap-v4/V4Quoter.json`
	- `abi/uniswap-v4/UniversalRouter.json`
- 来源说明：全部由 Etherscan 已验证主网合约导出。