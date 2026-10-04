# shelf

A tiny, zero-dependency library used as the training project of the genie
benchmark (see `bench/README.md` in the genie repository). It reads inventory
lines, keeps the items, computes the totals and exports the report.

## Usage

```js
import { parse, createStore, total, money, exportCsv } from "./src/index.js";

const store = createStore(parse("Огурцы;3;10;10\n# a comment\nМорковь;0;25"));
store.add({ name: "Репа", price: 30, quantity: 2, tags: ["овощи"] });

console.log(money(total(store.all())));
console.log(exportCsv(store.all()));
```

## The line format

`name;quantity;price[;discount]`. A line starting with `#` is a comment, blank
lines are skipped. `quantity` may be `0` (out of stock): it is a quantity, not a
missing value.

## Rules of the project

- Tests are plain `node --test`: `npm test` or `node --test`.
- The package is ESM (`"type": "module"`); modules export with `export function`.
- The library never writes to the console by itself — see `docs/logging.md`.
- Money is a number; `money()` in `src/format.js` is the only place that formats it.
