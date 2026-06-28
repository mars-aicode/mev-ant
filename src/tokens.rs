//! Token metadata and formatting utilities.
//!
//! This module centralises the list of tokens the scanner knows about
//! (symbols, decimals, supported profit tokens) plus the infrastructure
//! blacklist. Keeping this in one place makes it easy to add new tokens
//! or change decimals without hunting through `main.rs`.

use alloy::primitives::{Address, I256};

/// Token metadata: symbol, decimals.
#[derive(Debug, Clone, Copy)]
pub struct TokenMeta {
    pub symbol: &'static str,
    pub address: Address,
    pub decimals: u8,
}

pub const TOKEN_META: &[TokenMeta] = &[
    TokenMeta {
        symbol: "WETH",
        address: Address::new(hex_literal::hex!(
            "C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
        )),
        decimals: 18,
    },
    TokenMeta {
        symbol: "USDC",
        address: Address::new(hex_literal::hex!(
            "A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"
        )),
        decimals: 6,
    },
    TokenMeta {
        symbol: "USDT",
        address: Address::new(hex_literal::hex!(
            "dAC17F958D2ee523a2206206994597c13d831ec7"
        )),
        decimals: 6,
    },
    TokenMeta {
        symbol: "DAI",
        address: Address::new(hex_literal::hex!(
            "6B175474E89094C44Da98b954EedeAC495271d0F"
        )),
        decimals: 18,
    },
    TokenMeta {
        symbol: "WBTC",
        address: Address::new(hex_literal::hex!(
            "2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599"
        )),
        decimals: 8,
    },
    TokenMeta {
        symbol: "ETH",
        address: crate::models::ETH_TRANSFER_ADDR,
        decimals: 18,
    },
];

/// Default infrastructure blacklist — contracts that should never be candidates.
pub const DEFAULT_BLACKLIST: &[Address] = &[
    Address::new(hex_literal::hex!(
        "C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
    )),
    Address::new(hex_literal::hex!(
        "7a250d5630B4cF539739dF2C5dAcb4c659F2488D"
    )),
    Address::new(hex_literal::hex!(
        "E592427A0AEce92De3Edee1F18E0157C05861564"
    )),
    Address::new(hex_literal::hex!(
        "68b3465833fb72A70ecDF485E0e4C7bD8665Fc45"
    )),
    Address::new(hex_literal::hex!(
        "000000000004444c5dc75Cb358380D2e08dE62B0"
    )),
    Address::new(hex_literal::hex!(
        "BA12222222228d8Ba445958a75a0704d566BF2C8"
    )),
    Address::new(hex_literal::hex!(
        "1111111254EEB25477B68fb85Ed929f73A960582"
    )),
    Address::new(hex_literal::hex!(
        "111111125421cA6dc452d289314280a0f8842A65"
    )),
    Address::new(hex_literal::hex!(
        "C0FFEE0000000000000000000000000000000000"
    )),
];

/// Default supported tokens for profit calculation.
pub const DEFAULT_TOKENS: &[Address] = &[
    crate::models::ETH_TRANSFER_ADDR,
    Address::new(hex_literal::hex!(
        "C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
    )),
    Address::new(hex_literal::hex!(
        "dAC17F958D2ee523a2206206994597c13d831ec7"
    )),
    Address::new(hex_literal::hex!(
        "A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"
    )),
    Address::new(hex_literal::hex!(
        "6B175474E89094C44Da98b954EedeAC495271d0F"
    )),
    Address::new(hex_literal::hex!(
        "2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599"
    )),
];

/// Format a signed token amount with its symbol and decimals.
pub fn format_amount(amount: &I256, token: Address) -> String {
    let (sign, abs) = amount.into_sign_and_abs();
    let meta = TOKEN_META.iter().find(|m| m.address == token);
    let dec = meta.map(|m| m.decimals).unwrap_or(18);
    let sym = meta.map(|m| m.symbol).unwrap_or("???");
    let prefix = if sign.is_negative() { "-" } else { "" };
    let raw: u128 = abs.to::<u128>();
    let div = 10u128.pow(dec as u32);
    let int_part = raw / div;
    let frac = raw % div;
    format!(
        "{}{}.{:0>width$} {}",
        prefix,
        int_part,
        frac,
        sym,
        width = dec as usize
    )
}

/// Format a wei amount as ETH with 18 decimals.
pub fn format_wei(wei: u128) -> String {
    let dec = 18u32;
    let div = 10u128.pow(dec);
    let int_part = wei / div;
    let frac = wei % div;
    format!("{}.{:0>18} ETH", int_part, frac)
}
