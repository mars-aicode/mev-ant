# DODO

## 协议名称

- 协议名称：DODO
- 当前项目标签：`dodo`
- 当前支持：swap

## 合约地址

| 合约 | 地址 | 说明 |
| --- | --- | --- |
| DVMFactory | `0x72d220ce168c4f361dd4dee5d826a01ad8598f6c` | DODO V2 mainnet DVM 工厂，支持 `getDODOPool` / `getDODOPoolBidirection` |
| DPPFactory | `0xb5dc5e183c2acf02ab879a8569ab4edaf147d537` | DODO V2 mainnet DPP 工厂，支持 `getDODOPool` / `getDODOPoolBidirection` |
| DSPFactory | `0x3a97247df274a17c59a3bd12735ea3fcdfb49950` | DODO V2 mainnet DSP 工厂 |
| DVM Pool (WETH/USDC 示例) | `0xcfa990e9c104f6db3fbecee04ad211c39ed3830f` | 通过 `DVMFactory.getDODOPoolBidirection(WETH, USDC)` 实测取得的已验证主网池 |
| Pool | 动态地址 | 项目运行时通过 `log.address` 识别具体 DODO 池 |

## 文档地址

- 项目实现：`docs/project-implementation-guide.md`
- DODO 文档：<https://docs.dodoex.io/en/>
- 官方仓库：<https://github.com/DODOEX/contractV2>

## ABI 内容

- `docs/protocols/abi/dodo/DVMFactory.json`：主网已验证工厂 ABI，已确认包含 `getDODOPool(address,address)`、`getDODOPoolBidirection(address,address)`
- `docs/protocols/abi/dodo/DPPFactory.json`：主网已验证工厂 ABI，已确认包含 `getDODOPool(address,address)`、`getDODOPoolBidirection(address,address)`
- `docs/protocols/abi/dodo/DSPFactory.json`：主网已验证工厂 ABI
- `docs/protocols/abi/dodo/DVMPool_WETH_USDC.json`：代表性已验证池 ABI，已确认包含 `DODOSwap(address,address,uint256,uint256,address,address)`

当前项目的 DODO swap 解码只依赖 `DODOSwap` 事件；工厂 ABI 主要用于补充地址发现路径和协议参考。