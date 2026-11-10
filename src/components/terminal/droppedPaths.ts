const SHELL_METACHARACTERS = /[\s!"#$&'()*;<>?[\\\]^`{|}]/g;

/** Paths as Terminal.app types them on drop: shell metacharacters backslash-escaped. */
export const droppedPathsText = (paths: string[]) =>
  `${paths.map((path) => path.replace(SHELL_METACHARACTERS, "\\$&")).join(" ")} `;
