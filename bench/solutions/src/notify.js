export function notify(item, level = "info") {
  return { title: `Позиция «${item.name}»`, text: `Остаток: ${item.quantity}`, level };
}
