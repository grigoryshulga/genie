// Reading the lines of an inventory file.

/**
 * One line is `name;quantity;price[;discount]`; a line starting with `#` is a
 * comment and a blank line carries nothing (both give `null`).
 */
export function parseLine(line) {
  const text = line.trim();
  if (text === "" || text.startsWith("#")) return null;
  const fields = text.split(";").map((field) => field.trim());
  if (fields.length < 3) throw new Error(`too few fields: "${line}"`);
  if (fields.length > 4) throw new Error(`too many fields: "${line}"`);
  const [name, quantity, price, discount = "0"] = fields;
  if (name === "") throw new Error(`the name is empty: "${line}"`);
  return {
    name,
    quantity: number(quantity, "quantity"),
    price: number(price, "price"),
    discount: number(discount, "discount"),
  };
}

/** Every item of the text, in the order the lines come in. */
export function parse(text) {
  const items = [];
  for (const line of text.split("\n")) {
    const item = parseLine(line);
    if (item) items.push(item);
  }
  return items;
}

function number(value, what) {
  if (value === "") throw new Error(`the ${what} is empty`);
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || parsed < 0) throw new Error(`the ${what} is not a number: "${value}"`);
  return parsed;
}
