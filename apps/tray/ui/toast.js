'use strict'

const { invoke } = window.__TAURI__.core
const { listen } = window.__TAURI__.event
const appWindow = window.__TAURI__.window.getCurrentWindow()
const card = document.getElementById('toast-card')
let noticeId
let timer
let hovered = false
let revision = 0

function scheduleClose(ms = 8000) {
  clearTimeout(timer)
  if (hovered) return
  timer = setTimeout(() => closeToast().catch(console.error), ms)
}

function closeToast() {
  clearTimeout(timer)
  return appWindow.close()
}

function formatBody(raw) {
  const map = {
    same_category_and_campus: '同校区 · 时间相容'
  }
  return map[raw] || raw || ''
}

async function refresh() {
  const current = ++revision
  const { notice } = await invoke('get_bootstrap')
  if (current !== revision) return
  if (!notice) {
    await closeToast()
    return
  }
  noticeId = notice.id
  document.getElementById('toast-title').textContent = notice.title
  document.getElementById('toast-body').textContent = formatBody(notice.body)
  scheduleClose(8000)
}

card.addEventListener('mouseenter', () => {
  hovered = true
  clearTimeout(timer)
})
card.addEventListener('mouseleave', () => {
  hovered = false
  scheduleClose(3500)
})

document.getElementById('toast-close').addEventListener('click', event => {
  event.stopPropagation()
  closeToast().catch(console.error)
})
document.getElementById('toast-open').addEventListener('click', async () => {
  try {
    if (noticeId !== undefined) await invoke('open_notification', { noticeId })
    await closeToast()
  } catch (error) {
    console.error(error)
  }
})

listen('hub-event', () => refresh().catch(console.error))
  .then(refresh)
  .catch(console.error)
