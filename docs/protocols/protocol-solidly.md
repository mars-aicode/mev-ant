# Solidly 风格池

## 协议名称

- 协议名称：Solidly 风格 V2-like 池
- 当前项目标签：`solidly`
- 当前支持：swap、mint、burn

## 合约地址

| 合约 | 地址 | 说明 |
| --- | --- | --- |
| Pool | 动态地址 | 项目运行时通过 `log.address` 识别 |
| Factory | 动态地址 | 如果池子支持 `factory()`，可运行时读取 |
| Velodrome PoolFactory（参考实现，Optimism） | `0xF1046053aa5682b4F9a81b5481394DA16BE5FF5a` | 官方 README 部署地址；当前文档将其作为 Solidly 风格协议族的参考工厂 |

## 文档地址

- 项目实现：`docs/project-implementation-guide.md`
- 参考实现仓库：<https://github.com/velodrome-finance/contracts>
- 参考实现说明：<https://github.com/velodrome-finance/contracts/blob/main/README.md>
- 规范说明：<https://github.com/velodrome-finance/contracts/blob/main/SPECIFICATION.md>

## ABI 内容

- `docs/protocols/abi/solidly/IPool.json`：官方来源 `velodrome-finance/contracts` 的 `contracts/interfaces/IPool.sol`，已验证包含 `Swap(address,address,uint256,uint256,uint256,uint256)`、`Mint(address,uint256,uint256)`、`Burn(address,address,uint256,uint256)`、`token0()`、`token1()`、`factory()`、`stable()`
- `docs/protocols/abi/solidly/IPoolFactory.json`：官方来源 `velodrome-finance/contracts` 的 `contracts/interfaces/factories/IPoolFactory.sol`，已验证包含 `getPool(address,address,bool)`、`isPool(address)`、`implementation()`、`PoolCreated(address,address,bool,address,uint256)`

当前项目把 `solidly` 视为 ABI 兼容协议族而不是单一以太坊主网部署；具体 pool/factory 地址随兼容 fork 而变化，因此文档保留动态地址说明，并给出 Velodrome 作为官方参考实现。