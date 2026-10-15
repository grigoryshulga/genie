// The last work route: the settings mode's «← к работе» returns there (PD21).

const KEY = "genie:work-route";

/** Remember where the user was working, unless they are already in the settings mode. */
export function rememberWorkRoute(pathname: string, search: string): void {
  try {
    sessionStorage.setItem(KEY, pathname + search);
  } catch {
    // A full sessionStorage just means going back to the board.
  }
}

export function lastWorkRoute(): string {
  try {
    return sessionStorage.getItem(KEY) || "/board";
  } catch {
    return "/board";
  }
}
