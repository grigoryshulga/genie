// Reading the lines of an inventory file.
export function parseLine(line) {
  const text = line.trim();
  if (text === "" || text.startsWith("#")) return null;
  const fields = text.split(";").map((field) => field.trim());
  if (fields.length < 2) throw new Error(`too few fields: "${line}"`);
  if (fields.length > 4) throw new Error(`too many fields: "${line}"`);
  const [name, second, third, fourth] = fields;
  if (name === "") throw new Error(`the name is empty: "${line}"`);
  const two = fields.length === 2;
  return {
    name,
    quantity: two ? 1 : number(second, "quantity"),
    price: number(two ? second : third, "price"),
    discount: number(two ? "0" : (fourth ?? "0"), "discount"),
  };
}
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
