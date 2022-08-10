import type { ReactNode } from 'react';

interface Props {
  children?: ReactNode;
  type: 'submit' | 'button';
}

export function Button({ children, ...props }: Props) {
  return (
    <button
      {...props}
      className='group relative flex w-full justify-center rounded-md border border-transparent bg-indigo-600 py-2 px-4 text-sm font-medium text-white hover:bg-indigo-700 focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:ring-offset-2'
    >
      {children}
    </button>
  );
}

export default Button;
