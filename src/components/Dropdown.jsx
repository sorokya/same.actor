export function Dropdown({ items, addSelection }) {
  return (
    <ul className="mt-2 mb-4">
      {items.map((item) => (
        <li
          key={item.id}
          className="flex p-1 hover:cursor-pointer bg-slate-100 even:bg-slate-200"
          onClick={() => addSelection(item)}
        >
          <img
            src={item.imgUrl}
            alt={`${item.name} poster`}
            className="inline-block h-20"
          />
          <div className="ml-1 inline-block">
            <span className="text-xl text-slate-800">
              {item.name}
              {item.releaseYear ? ` (${item.releaseYear})` : null}
            </span>
            <p className="text-slate-800">
              {item.overview.length > 150
                ? `${item.overview.substring(0, 150)}...`
                : item.overview}
            </p>
          </div>
        </li>
      ))}
    </ul>
  );
}
