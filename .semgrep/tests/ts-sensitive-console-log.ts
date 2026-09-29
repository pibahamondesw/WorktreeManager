declare const linearToken: string;
declare const config: { apiKey: string };
declare const error: unknown;

// ruleid: ts-sensitive-console-log
console.log("token", linearToken);
// ruleid: ts-sensitive-console-log
console.error(config.apiKey);

// ok: ts-sensitive-console-log
console.error("Update check failed:", error);
// ok: ts-sensitive-console-log
console.log("tokenizer ready");
