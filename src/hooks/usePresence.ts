import { AnimationEvent, useState } from "react";

export function usePresence(visible: boolean) {
  const [rendered, setRendered] = useState(visible);
  if (visible && !rendered) setRendered(true);

  const onAnimationEnd = (event: AnimationEvent) => {
    if (event.target === event.currentTarget && !visible) setRendered(false);
  };

  return { rendered, exiting: rendered && !visible, onAnimationEnd };
}
