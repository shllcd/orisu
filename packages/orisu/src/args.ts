type Option = { short?: string; description: string; default?: unknown };

export const formatOptions = (options: Record<string, Option>, dim = (s: string) => s): string[] =>
  Object.entries(options).map(([name, option]) => {
    const flag = (option.short ? `-${option.short}, ` : "    ") + `--${name}`;
    const fallback = option.default === undefined ? "" : ` ${dim(`[default: ${option.default}]`)}`;
    return `  ${flag.padEnd(18)}${dim("·".repeat(4))} ${option.description}${fallback}`;
  });
