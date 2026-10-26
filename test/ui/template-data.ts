import { newTemplate } from "../../web/src/entities/agent-config/model.ts";
const title = "Проверка интеграций и совместимости нескольких сервисов";
export const TEMPLATE_FILE = {
  ...newTemplate(title, "Команда с длинным названием, персональными инструкциями и явными связями."),
  title,
  members: [
    { role: "executor", name: "bender", thinking: "high", instructions: "Preserve compatibility and attach evidence.", model: "anthropic/claude-sonnet-4-6" },
    { role: "reviewer", name: "watson", instructions: "Review the result against the acceptance criteria." },
  ],
  charter: "## Общие правила\n\n- Сохраняйте совместимость.\n- Прикладывайте результаты проверок.",
};
