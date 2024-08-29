import { Button } from "./Button";

export function FindOut({ onClick }) {
  return (
    <Button
      onClick={onClick}
      className="p-1 ml-1 text-2xl bg-indigo-400 rounded hover:bg-indigo-500 hover:cursor-pointer"
    >
      Find out!
    </Button >
  )
}
