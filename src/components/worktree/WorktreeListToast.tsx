interface WorktreeListToastProps {
  message: string;
}

export function WorktreeListToast({ message }: WorktreeListToastProps) {
  return (
    <div className="absolute bottom-16 left-1/2 -translate-x-1/2 px-4 py-2 bg-bg-active border border-border rounded-lg text-xs text-text-primary shadow-xl animate-fade-in select-text">
      {message}
    </div>
  );
}
