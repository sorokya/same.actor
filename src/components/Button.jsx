import { useMemo } from "react";

export function Button({ children, variant, ...props }) {
  const { disabled } = props

  const colors = useMemo(() => {
    switch (variant) {
      case 'danger':
        return 'from-red-600 to-pink-500 focus:ring-pink-300 dark:focus:ring-pink-800';
      default:
        return 'from-purple-600 to-blue-500 focus:ring-blue-300 dark:focus:ring-blue-800';
    }
  }, [variant]);

  const styles = useMemo(() => {
    let styles = `w-52 text-white bg-gradient-to-br hover:bg-gradient-to-bl focus:ring-4 focus:outline-none font-medium rounded-lg text-sm px-5 py-2.5 text-center me-2 mb-2 ${colors}`;

    if (disabled) {
      styles += ' opacity-50 cursor-not-allowed';
    }

    return styles;
  }, [disabled, colors]);

  if (props.disabled) {
  }

  return (<button {...props} className={styles}>{children}</button>);
}
