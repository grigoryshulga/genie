import { readFileSync } from "node:fs";
import { join } from "node:path";
import { ModelRuntime } from "@earendil-works/pi-coding-agent";

function send(type: string, data: Record<string, unknown>) {
  process.stdout.write(`${JSON.stringify({ type, ...data })}\n`);
}

function clean(value: unknown): string {
  return String(value ?? "")
    .replace(/(?:Bearer\s+|sk-)[^\s"']+/gi, "[redacted]")
    .replace(/\b(access_token|refresh_token|id_token|code_verifier)\s*[:=]\s*[^\s,;]+/gi, "$1=[redacted]")
    .replace(/\beyJ[a-zA-Z0-9._-]{30,}/g, "[redacted]")
    .slice(0, 2000);
}

export default function (pi: any) {
  pi.registerCommand("genie-oauth-connect", {
    description: "Connect a provider to the user's Genie profile",
    handler: async (_args: string, ctx: any) => {
      const provider = process.env.GENIE_PROVIDER ?? "";
      const dir = process.env.PI_CODING_AGENT_DIR ?? "";
      const authPath = join(dir, "auth.json");
      try {
        const runtime = await ModelRuntime.create({ authPath, modelsPath: join(dir, "models.json") });
        await runtime.login(provider, "oauth", {
          prompt: async (prompt) => {
            if (prompt.type === "select") {
              const options = prompt.options ?? [];
              const selected = await ctx.ui.select(clean(prompt.message), options.map((option) => option.label));
              if (selected === undefined) throw new Error("OAuth prompt was cancelled");
              return options.find((option) => option.label === selected)?.id ?? "";
            }
            const title = prompt.type === "secret" ? `[genie-secret] ${clean(prompt.message)}` : clean(prompt.message);
            const value = await ctx.ui.input(title, clean(prompt.placeholder));
            if (value === undefined) throw new Error("OAuth prompt was cancelled");
            return value;
          },
          notify: (event) => {
            if (event.type === "auth_url") {
              send("genie_oauth_authorization", {
                url: event.url,
                instructions: event.instructions,
              });
            } else if (event.type === "device_code") {
              send("genie_oauth_authorization", { url: event.verificationUri, code: event.userCode });
            } else if (event.type === "info" && event.links?.length) {
              send("genie_oauth_authorization", { url: event.links[0].url, instructions: event.message });
            } else {
              send("genie_oauth_progress", { message: clean(event.message) });
            }
          },
        });
        const profile = JSON.parse(readFileSync(authPath, "utf8"));
        if (!profile[provider]) throw new Error("Pi завершил OAuth, но данные провайдера не появились");
        send("genie_oauth_result", { provider, auth: profile[provider] });
      } catch (error) {
        send("genie_oauth_error", { message: clean(error instanceof Error ? error.message : error) || "OAuth не завершён" });
      }
    },
  });
}
