import { Search } from "lucide-react";
import { useApp } from "../store";

export function SearchBar() {
  const { t, searchQuery, setSearchQuery } = useApp();

  return (
    <div className="relative w-64 group mr-4 shrink-0 hidden md:block">
      <Search className="w-4 h-4 text-black/40 dark:text-white/40 absolute left-3 top-1/2 -translate-y-1/2 group-focus-within:text-blue-500 transition-colors" />
      <input
        type="text"
        placeholder={t.search}
        value={searchQuery}
        onChange={(e) => setSearchQuery(e.target.value)}
        className="w-full bg-black/5 dark:bg-white/5 border border-black/10 dark:border-white/10 rounded-lg py-2 pl-9 pr-4 text-xs font-bold tracking-widest text-black dark:text-white placeholder:text-black/40 dark:placeholder:text-white/40 focus:outline-none focus:border-blue-500/50 transition-all font-sans"
      />
    </div>
  );
}
