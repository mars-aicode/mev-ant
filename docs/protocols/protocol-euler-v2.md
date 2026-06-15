# Euler V2

## 协议名称

- 协议名称：Euler V2
- 当前项目标签：`euler_v2_evault`
- 当前支持：liquidation

## 合约地址

| 合约 | 地址 | 说明 |
| --- | --- | --- |
| eVault | 动态地址 | 项目从 `trace.to` 识别 |

## 文档地址

- 项目实现：`docs/project-implementation-guide.md`
- Euler 文档：<https://docs.euler.finance/>
- 官方仓库：<https://github.com/euler-xyz/euler-interfaces>

## ABI 内容

- `docs/protocols/abi/euler-v2/EVault.json`
	- 官方来源：`euler-xyz/euler-interfaces` 的 `abis/EVault.json`
	- 已验证包含：`liquidate(address,address,uint256,uint256)`

Euler V2 的 eVault 地址按市场动态部署；当前项目仍通过 `trace.to` 识别具体 vault。