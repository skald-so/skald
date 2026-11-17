// The space the cabinet's switcher chose, for the sections of one space.
import { createContext } from "react";
import type { ListedSpace } from "../api/skald.schemas.ts";

/** The space the switcher chose; none while the account has no space. */
export const ChosenSpace = createContext<ListedSpace | undefined>(undefined);
