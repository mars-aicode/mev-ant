# Maker

## 协议名称

- 协议名称：Maker
- 当前项目标签：`maker_dog`、`maker_clipper`
- 当前支持：liquidation

## 合约地址

| 合约 | 地址 | 说明 |
| --- | --- | --- |
| Dog | `0x135954d155898D42C90D2a57824C690e0c7BEf1B` | Maker mainnet 全局清算入口，来自官方 chainlog `MCD_DOG` |
| Clipper (ETH-A 示例) | `0xc67963a226eddd77B91aD8c421630A1b0AdFF270` | Maker mainnet ETH-A 拍卖合约，来自官方 chainlog `MCD_CLIP_ETH_A` |
| Clipper (其他抵押品) | 动态地址 | 各 collateral 对应不同 `MCD_CLIP_*`，项目仍从 `trace.to` 识别 |

## 文档地址

- 项目实现：`docs/project-implementation-guide.md`
- Maker 文档：<https://docs.makerdao.com/>
- Maker mainnet chainlog API：<https://chainlog.sky.money/api/mainnet/active.json>

## ABI 内容

- `docs/protocols/abi/maker/Dog.json`
	- 已验证包含：`bark(bytes32,address,address)`
- `docs/protocols/abi/maker/ClipperETHA.json`
	- 已验证包含：`take(uint256,uint256,uint256,address,bytes)`

当前项目只依赖上面两个方法签名；`Clipper` 的完整 ABI 采用 ETH-A 拍卖合约作为代表样本，其他 `MCD_CLIP_*` 合约通常复用同一接口表面。