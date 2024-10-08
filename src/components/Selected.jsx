export function Selected({ selection, removeSelection }) {
  return (
    <ul className="flex flex-row gap-2">
      {selection.map((item) => (
        <li
          key={item.id}
          onClick={() => removeSelection(item)}
          title="Click to remove"
          className="p-1 bg-slate-100 text-slate-800 rounded hover:cursor-pointer relative"
        >
          <div>
            {item.name}
            {item.releaseYear ? ` (${item.releaseYear})` : null}
          </div>
        </li>
      ))}
    </ul>
  );
}
