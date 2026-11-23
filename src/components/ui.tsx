import type { ButtonHTMLAttributes, ReactNode } from 'react';
import { FaSpinner } from 'react-icons/fa';
import type { Mode } from '../lib/types';

type Variant = 'primary' | 'ghost' | 'danger';

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  busy?: boolean;
  children: ReactNode;
}

const VARIANTS: Record<Variant, string> = {
  primary:
    'bg-accent text-ink hover:bg-[#e6b355] disabled:bg-raise-2 disabled:text-dim disabled:border disabled:border-line',
  ghost: 'border border-line text-paper hover:bg-raise',
  danger: 'border border-line text-dim hover:border-danger hover:text-danger',
};

export function Button({
  variant = 'primary',
  busy = false,
  className = '',
  children,
  ...props
}: ButtonProps) {
  return (
    <button
      type="button"
      // Every control is a real button: focusable, Enter/Space works, and screen readers
      // announce it as a button. The previous UI used onClick on <li> and <div>.
      {...props}
      disabled={props.disabled === true || busy}
      className={`inline-flex items-center justify-center gap-2 rounded px-4 py-2 text-sm font-medium transition-colors ${VARIANTS[variant]} ${className}`}
    >
      {busy ? <FaSpinner className="animate-spin" aria-hidden="true" /> : null}
      {children}
    </button>
  );
}

interface ModeSwitchProps {
  mode: Mode;
  onChange: (mode: Mode) => void;
}

const MODES: ReadonlyArray<{ value: Mode; label: string; hint: string }> = [
  { value: 'title', label: 'Titles', hint: 'Compare films and shows, find shared cast' },
  { value: 'person', label: 'People', hint: 'Compare actors, find shared titles' },
];

export function ModeSwitch({ mode, onChange }: ModeSwitchProps) {
  return (
    <fieldset className="inline-flex rounded-full border border-line bg-raise p-0.5">
      <legend className="sr-only">What are you comparing?</legend>
      {MODES.map((option) => (
        <label
          key={option.value}
          title={option.hint}
          className="relative cursor-pointer rounded-full"
        >
          {/* A real radio: arrow keys, grouping and the checked state all come free from the
              platform. The previous control faked a radiogroup out of buttons. */}
          <input
            type="radio"
            name="compare-mode"
            value={option.value}
            checked={mode === option.value}
            onChange={() => onChange(option.value)}
            className="peer sr-only"
          />
          <span className="block rounded-full px-3 py-1 font-mono text-[11px] tracking-[0.12em] uppercase text-dim transition-colors peer-checked:bg-paper peer-checked:text-ink peer-hover:text-paper peer-checked:peer-hover:text-ink peer-focus-visible:outline-2 peer-focus-visible:outline-offset-2 peer-focus-visible:outline-accent">
            {option.label}
          </span>
        </label>
      ))}
    </fieldset>
  );
}
