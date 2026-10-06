import { ChatImage } from "../../services/chat";

export interface ImageAttachment extends ChatImage {
  id: string;
  name: string;
  url: string;
}

const SUPPORTED_IMAGE_TYPES = ["image/png", "image/jpeg", "image/gif", "image/webp"];
const MAX_IMAGE_BYTES = 10 * 1024 * 1024;

export const imageFiles = (files: FileList | File[] | null | undefined) =>
  Array.from(files ?? []).filter((file) => SUPPORTED_IMAGE_TYPES.includes(file.type));

export const oversizedImages = (files: File[]) =>
  files.filter((file) => file.size > MAX_IMAGE_BYTES);

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
