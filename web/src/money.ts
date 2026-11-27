/** Micro-USD (1,000,000 = $1) as dollars; sub-cent amounts keep four decimals. */
export const usd = (micro: number) => `$${(micro / 1e6).toFixed(micro !== 0 && micro < 10_000 ? 4 : 2)}`
