import { SearchBar } from "./SearchBar";
import { ActiveThreads } from "./ActiveThreads";
import { StorageStats } from "./StorageStats";
import { NewTaskButton } from "./NewTaskButton";

export function Header() {
  return (
    <header className="h-20 border-b border-black/10 dark:border-white/10 flex items-center justify-between px-8 bg-black/5 dark:bg-black/20 shrink-0">
      <div className="flex gap-8 items-center flex-1 min-w-[520px] shrink-0">
        <SearchBar />

        <div className="w-px h-8 bg-black/10 dark:bg-white/10 hidden md:block"></div>

        <div className="flex items-center gap-6 shrink-0">
          <ActiveThreads />
          <div className="w-px h-4 bg-black/10 dark:bg-white/10"></div>
          <StorageStats />
        </div>
      </div>

      <div className="flex items-center gap-4 pl-4 shrink-0">
        <NewTaskButton />
      </div>
    </header>
  );
}
