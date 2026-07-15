import { defineConfig } from 'vite'
import { resolve } from 'path'

export default defineConfig({
  base: '/DevDeck/',
  build: {
    rollupOptions: {
      input: {
        main: resolve(__dirname, 'index.html'),
        download: resolve(__dirname, 'download.html'),
        help: resolve(__dirname, 'help.html'),
        about: resolve(__dirname, 'about.html')
      }
    }
  }
})
