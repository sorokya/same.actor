export function Footer() {
  return (
    <footer className="w-full p-2 bg-slate-500 border-t border-slate-300 shadow md:flex md:items-center md:justify-between">
      <div className="text-sm text-slate-50 text-center md:text-left sm:w-full">© 2024 Richard Leek. All Rights Reserved.</div>
      <div className="flex flex-wrap items-center mt-3 text-center font-medium text-slate-50 sm:mt-0">
        <p className="text-xs m-auto">
          <img src="https://www.themoviedb.org/assets/2/v4/logos/v2/blue_long_2-9665a76b1ae401a510ec1e0ca40ddcb3b0cfe45f1d51b77a308fea0845885648.svg" className="w-full h-3" />
          This product uses the TMDB API but is not endorsed or certified by TMDB.
        </p>
      </div>
    </footer>
  )
}
