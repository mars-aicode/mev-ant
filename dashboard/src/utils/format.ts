const TOKEN_META: Record<string, { sym: string; dec: number }> = {
  '0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2': { sym: 'WETH', dec: 18 },
  '0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48': { sym: 'USDC', dec: 6 },
  '0xdac17f958d2ee523a2206206994597c13d831ec7': { sym: 'USDT', dec: 6 },
  '0x6b175474e89094c44da98b954eedeac495271d0f': { sym: 'DAI', dec: 18 },
  '0x2260fac5e5542a773aa44fbcfedf7c193bc2c599': { sym: 'WBTC', dec: 8 },
  '0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee': { sym: 'ETH', dec: 18 },
};

function fracDigits(dec: number) { return dec >= 18 ? 10 : dec; }

function formatTokenAmount(amount: string, meta: { sym: string; dec: number }): string {
  const raw = String(amount);
  const padded = raw.padStart(meta.dec + 1, '0');
  const intPart = padded.slice(0, padded.length - meta.dec) || '0';
  const fracPart = padded.slice(-meta.dec).slice(0, fracDigits(meta.dec));
  return `${intPart}.${fracPart} ${meta.sym}`;
}

/** Return symbol for a known token address, or null if unknown */
export function tokenSymbol(addr: string): string | null {
  const meta = TOKEN_META[(addr || '').toLowerCase()];
  return meta ? meta.sym : null;
}

/** Convert hex amount string to formatted "x.xx SYM" */
export function formatAmount(token: string, hexAmount: string): string {
  try {
    const addr = (token || '').toLowerCase();
    const meta = TOKEN_META[addr];
    // Parse hex to decimal string (right-to-left carry propagation)
    const hex = hexAmount.startsWith('0x') ? hexAmount.slice(2) : hexAmount;
    let digits: number[] = [];
    for (let ci = 0; ci < hex.length; ci++) {
      let carry = parseInt(hex[ci], 16);
      for (let i = digits.length - 1; i >= 0; i--) {
        const v = digits[i] * 16 + carry;
        digits[i] = v % 10;
        carry = Math.floor(v / 10);
      }
      while (carry > 0) {
        digits.unshift(carry % 10);
        carry = Math.floor(carry / 10);
      }
    }
    const decStr = digits.length > 0 ? digits.join('') : '0';
    if (!meta) return `${decStr} ???`;
    return formatTokenAmount(decStr, meta);
  } catch {
    return `${hexAmount} ???`;
  }
}

/** Format wei amount as "x.xxxxxxxxxx ETH" */
export function formatEth(wei: number | string): string {
  const raw = String(wei);
  const padded = raw.padStart(19, '0');
  const intPart = padded.slice(0, padded.length - 18) || '0';
  const fracPart = padded.slice(-18).slice(0, 10);
  return `${intPart}.${fracPart} ETH`;
}

/** Parse profit_json string into "x.xx SYM x.xx SYM" format */
export function formatProfit(profitJson: string): string {
  try {
    const items = JSON.parse(profitJson);
    if (!Array.isArray(items) || items.length === 0) return '-';
    return items.map((i: any) => {
      const token = (i.token || '').toLowerCase();
      const meta = TOKEN_META[token];
      if (!meta) return `${i.amount} ???`;
      return formatTokenAmount(String(i.amount), meta);
    }).join(' ');
  } catch {
    return profitJson;
  }
}
