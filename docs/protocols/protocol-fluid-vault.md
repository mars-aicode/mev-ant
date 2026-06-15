# Fluid Vault

## 协议名称

- 协议名称：Fluid Vault
- 当前项目标签：`fluid_vault_t1`、`fluid_vault_t2_t3`、`fluid_vault_t4`
- 当前支持：liquidation

## 合约地址

| 合约 | 地址 | 说明 |
| --- | --- | --- |
| Vault | 动态地址 | 项目从 `trace.to` 识别 |

## 文档地址

- 项目实现：`docs/project-implementation-guide.md`
- 项目补充：`crates/block-pipeline/src/liquidation_enrichment.rs`
- 官方仓库：<https://github.com/Instadapp/fluid-contracts-public>

## ABI 内容

- `docs/protocols/abi/fluid-vault/IFluidVault.json`
	- 官方来源：`Instadapp/fluid-contracts-public` 的 `contracts/protocols/vault/interfaces/iVault.sol`
	- 已验证包含：`TYPE()`、`constantsView()`
- `docs/protocols/abi/fluid-vault/IFluidVaultT1.json`
	- 官方来源：`contracts/protocols/vault/interfaces/iVaultT1.sol`
	- 已验证包含：`liquidate(uint256,uint256,address,bool)`、`constantsView()`
- `docs/protocols/abi/fluid-vault/IFluidVaultT2.json`
	- 官方来源：`contracts/protocols/vault/interfaces/iVaultT2.sol`
	- 已验证包含：`liquidate(uint256,uint256,uint256,uint256,address,bool)`、`liquidatePerfect(uint256,uint256,uint256,uint256,address,bool)`、`TYPE()`、`constantsView()`
- `docs/protocols/abi/fluid-vault/IFluidVaultT3.json`
	- 官方来源：`contracts/protocols/vault/interfaces/iVaultT3.sol`
	- 已验证包含：`liquidate(uint256,uint256,uint256,uint256,address,bool)`、`liquidatePerfect(uint256,uint256,uint256,uint256,address,bool)`、`TYPE()`、`constantsView()`
- `docs/protocols/abi/fluid-vault/IFluidVaultT4.json`
	- 官方来源：`contracts/protocols/vault/interfaces/iVaultT4.sol`
	- 已验证包含：`liquidate(uint256,uint256,uint256,uint256,uint256,uint256,address,bool)`、`liquidatePerfect(uint256,uint256,uint256,uint256,uint256,uint256,address,bool)`、`TYPE()`、`constantsView()`

Fluid Vault 地址按工厂动态部署
