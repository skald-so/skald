import styles from "./FilterButtons.module.css";

export interface FilterOption<Value extends string> {
  value: Value;
  label: string;
}

export interface FilterButtonsProps<Value extends string> {
  /** The accessible name of the group, as "Filter accounts". */
  label: string;
  options: readonly FilterOption<Value>[];
  /** The active choice, set by the screen. */
  value: Value;
  /** A press on another choice: its value. A press on the active one reports nothing. */
  onChange: (value: Value) => void;
  /** One frame around the choices, as "Thread · Graph"; separate buttons otherwise. */
  joined?: boolean;
}

/**
 * Filter buttons of the design screens, with one active choice, as "All · Flagged · Banned".
 * Each choice is a toggle button: a screen reader hears it as pressed when it is active.
 */
export function FilterButtons<Value extends string>({
  label,
  options,
  value,
  onChange,
  joined,
}: FilterButtonsProps<Value>) {
  return (
    <div
      role="group"
      aria-label={label}
      className={[styles.group, joined ? styles.joined : styles.separate].join(
        " ",
      )}
    >
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          className={styles.choice}
          aria-pressed={option.value === value}
          onClick={() => {
            if (option.value !== value) onChange(option.value);
          }}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}
