import { ChatImage } from "../../services/chat";

export interface ImageAttachment extends ChatImage {
  id: string;
  name: string;
  url: string;
}

export const imageFiles = (files: FileList | null | undefined) =>
  Array.from(files ?? []).filter((file) => file.type.startsWith("image/"));

export function readImage(file: File): Promise<ImageAttachment> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(reader.error ?? new Error(`Could not read ${file.name}`));
    reader.onload = () => {
      const url = String(reader.result);
      resolve({
        id: crypto.randomUUID(),
        name: file.name || "Pasted image",
        mediaType: file.type,
        data: url.slice(url.indexOf(",") + 1),
        url,
      });
    };
    reader.readAsDataURL(file);
  });
}
