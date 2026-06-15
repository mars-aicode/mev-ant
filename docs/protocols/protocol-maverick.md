# Maverick

## 协议名称

- 协议名称：Maverick
- 当前项目标签：`maverick`、`maverick_v2`
- 当前支持：swap

## 合约地址

| 合约 | 地址 | 说明 |
| --- | --- | --- |
| Pool | 动态地址 | 项目运行时通过 `log.address` 识别 |

## 文档地址

- 项目实现：`docs/project-implementation-guide.md`
- Maverick 文档：<https://docs.mav.xyz/>
- 官方仓库：<https://github.com/maverickprotocol/maverick-v1-interfaces>
- 官方仓库：<https://github.com/maverickprotocol/v2-interfaces>

## ABI 内容

- `docs/protocols/abi/maverick/IPool.json`
	- 官方来源：`maverickprotocol/maverick-v1-interfaces` 的 `abi/IPool.json`
	- 已验证包含：`Swap(address,address,bool,bool,uint256,uint256,int32)`
- `docs/protocols/abi/maverick/MaverickV2Pool.json`
	- 官方来源：`maverickprotocol/v2-interfaces` 的 `abis/MaverickV2Pool.json`
	- 已验证包含：`PoolSwap(address,address,(uint256,bool,bool,int32),uint256,uint256)`

Maverick v1/v2 池地址按市场动态部署；
