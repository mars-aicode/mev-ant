# 协议 ABI 与部署地址参考

这份文档现在作为索引页使用。每个协议或协议族都单独拆成了独立文件，统一按下面四类信息整理：

- 协议名称
- 合约地址
- 文档地址
- ABI 内容 / JSON 文件

默认语境仍然是 Ethereum mainnet。对没有固定 singleton / factory 的协议，单独文件里会明确标成“动态地址发现”。

## 使用约定

- 同一个协议族下的明显变体会合并到一个文件。
  例子：`Aave` 合并标准池和 `L2Pool`，`Maker` 合并 `Dog` / `Clipper`，`Fluid Vault` 合并 T1/T2/T3/T4。
- clone 协议会单独成文件，但 ABI 内容通常会直接链接到共享的上游完整 JSON ABI。
- 完整 ABI JSON 默认保存在 `docs/protocols/abi/` 下；少数协议还会额外补充 wire format / trace 格式说明。Ekubo 已有可导出的 Core ABI，但当前项目的 swap 解码仍依赖它的 zero-selector / zero-topic 特征。

## AMM 与流动性协议

- [Uniswap V2](protocol-uniswap-v2.md)
- [Sushi](protocol-sushi.md)
- [Shiba V1](protocol-shiba-v1.md)
- [Saita](protocol-saita.md)
- [Pancake V2](protocol-pancake-v2.md)
- [Solidly 风格池](protocol-solidly.md)
- [Uniswap V3](protocol-uniswap-v3.md)
- [Shiba V2](protocol-shiba-v2.md)
- [Pancake V3](protocol-pancake-v3.md)
- [Uniswap V4](protocol-uniswap-v4.md)
- [Balancer](protocol-balancer.md)
- [Curve AMM](protocol-curve.md)
- [DODO](protocol-dodo.md)
- [Maverick](protocol-maverick.md)
- [Ekubo](protocol-ekubo.md)

## 清算协议

- [Aave](protocol-aave.md)
- [Compound V2](protocol-compound-v2.md)
- [Compound V3 Comet](protocol-compound-v3-comet.md)
- [Maker](protocol-maker.md)
- [Liquity V1](protocol-liquity-v1.md)
- [Morpho Blue](protocol-morpho-blue.md)
- [Fraxlend](protocol-fraxlend.md)
- [Fluid Vault](protocol-fluid-vault.md)
- [Euler V2](protocol-euler-v2.md)
- [Curve LlamaLend](protocol-curve-llamalend.md)
