'use strict'

const { invoke } = window.__TAURI__.core
const $ = id => document.getElementById(`campus-${id}`)

const services = {
  mail: { name: '邮件', source: '学校 Outlook', guide: '在学校 Outlook 登录后查询邮件。', query: '查询邮件' },
  timetable: { name: '课表', source: '教务 SIS', guide: '在教务 SIS 或科大 Go 查看个人课表。', query: '查询课表' },
  swimming: { name: '游泳馆', source: '科大 Go', guide: '打开微信「科大 Go」，进入「体育馆预约」并选择游泳馆。', query: '查询时段', bookable: true },
  'meeting-room': { name: '会议室', source: '学校 Booking', guide: '在学校 Booking 选择会议室和时段。可用房间取决于账号权限。', query: '查询时段', bookable: true }
}
const campusZone = 'Asia/Shanghai'
const dayFormatter = new Intl.DateTimeFormat('zh-CN', { timeZone: campusZone, month: 'numeric', day: 'numeric' })
const timeFormatter = new Intl.DateTimeFormat('zh-CN', { timeZone: campusZone, hour: '2-digit', minute: '2-digit', hourCycle: 'h23' })
let selected = 'mail'
let status = null
let busy = false
let preview = null

function element(tag, text, className) {
  const node = document.createElement(tag)
  if (text) node.textContent = text
  if (className) node.className = className
  return node
}

function campusDate() {
  const formatter = new Intl.DateTimeFormat('en', { timeZone: campusZone, year: 'numeric', month: '2-digit', day: '2-digit' })
  const parts = Object.fromEntries(formatter.formatToParts(new Date()).map(part => [part.type, part.value]))
  return `${parts.year}-${parts.month}-${parts.day}`
}

function timestamp(raw, includeDate = false) {
  if (raw == null || raw === '') return ''
  if (typeof raw === 'string' && /^\d\d:\d\d(?::\d\d)?$/.test(raw)) return raw.slice(0, 5)
  const value = typeof raw === 'string' && /^\d{4}-\d{2}-\d{2}T/.test(raw) && !/(?:z|[+-]\d{2}:?\d{2})$/i.test(raw)
    ? `${raw}+08:00` : raw
  const date = new Date(typeof value === 'number' ? value * 1000 : value)
  if (!Number.isFinite(date.getTime())) return ''
  return includeDate ? `${dayFormatter.format(date)} ${timeFormatter.format(date)}` : timeFormatter.format(date)
}

function timeRange(slot, includeDate = false) {
  return [timestamp(slot.start, includeDate), timestamp(slot.end)].filter(Boolean).join(' – ')
}

function serviceStatus(snapshot = status) {
  return snapshot?.services.find(service => service.id === selected)
}

function feedback(text = '', error = false) {
  const line = $('feedback')
  line.textContent = text
  line.hidden = !text
  line.classList.toggle('error', error)
}

function renderService() {
  const service = services[selected]
  const info = serviceStatus()
  const demo = status?.mode === 'demo'
  const connected = info?.connected === true
  const account = { student: '学生', staff: '教职工' }[info?.account] || info?.account
  $('service-name').textContent = info?.name || service.name
  $('source').textContent = `来源：${info?.source || service.source}`
  $('connection').textContent = status
    ? `${demo ? '模拟' : ''}${connected ? '已连接' : demo ? '未连接' : '未授权'}${connected && account ? ` · ${account}` : ''}`
    : busy ? '读取中' : '状态暂不可用'
  $('handoff').textContent = service.guide
  $('handoff').hidden = demo
  $('connect').hidden = !demo || connected
  $('disconnect').hidden = !connected
  $('query').hidden = !connected
  $('observed').hidden = !connected || !$('observed').textContent
  $('query-button').textContent = service.query
  $('date-field').hidden = selected === 'mail'
  $('date').required = selected !== 'mail' && connected
  $('unread-field').hidden = selected !== 'mail'
  $('bookings').hidden = !service.bookable
  for (const button of document.querySelectorAll('[data-service]')) {
    button.setAttribute('aria-pressed', String(button.dataset.service === selected))
    button.disabled = busy
  }
  for (const id of ['official', 'connect', 'disconnect', 'query-button', 'bookings', 'date', 'unread']) {
    $(id).disabled = busy
  }
  for (const button of document.querySelectorAll('.campus-row-action')) button.disabled = busy
}

function clearResults() {
  $('results').replaceChildren()
  $('results').hidden = true
  $('observed').textContent = ''
  $('observed').hidden = true
}

function clearPrivateView() {
  clearResults()
  $('receipt').replaceChildren()
  $('receipt').hidden = true
  preview = null
  if ($('confirmation').open) $('confirmation').close()
  $('confirmation-detail').replaceChildren()
}

function acceptStatus(next) {
  const previous = serviceStatus()
  const current = serviceStatus(next)
  if (status?.mode !== next.mode || previous?.account !== current?.account || previous?.connected !== current?.connected) {
    clearPrivateView()
  }
  status = next
}

function request(action, payload = {}) {
  return invoke('campus_assistant', { action, payload })
}

async function perform(work, onResult, quiet = false) {
  if (busy) return
  busy = true
  renderService()
  if (!quiet) feedback('处理中…')
  try {
    const result = await work()
    if (!quiet) feedback()
    onResult(result)
  } catch (error) {
    feedback(typeof error === 'string' ? error : error?.message || '操作失败，请稍后重试。', true)
  } finally {
    busy = false
    renderService()
  }
}

function row(title, details, badge) {
  const card = element('article', '', 'card')
  const header = element('div', '', 'campus-row-head')
  header.append(element('strong', title))
  if (badge) header.append(element('span', badge, 'pill'))
  card.append(header)
  if (details) card.append(element('span', details, 'campus-row-meta'))
  return card
}

function addAction(card, label, action) {
  const button = element('button', label, 'btn compact campus-row-action')
  button.type = 'button'
  button.addEventListener('click', action)
  card.append(button)
}

function showResults(rows, emptyMessage, observedAt) {
  $('results').replaceChildren(...(rows.length ? rows : [element('p', emptyMessage, 'empty-box')]))
  $('results').hidden = false
  const observed = timestamp(observedAt, true)
  $('observed').textContent = observed ? `查询时间（广州） ${observed}` : ''
}

function showMail(result) {
  const rows = (result.messages || []).map(message => {
    const sender = typeof message.from === 'string' ? message.from : message.from?.name || message.from?.address
    const details = [sender, timestamp(message.receivedAt, true)].filter(Boolean).join(' · ')
    const card = row(message.subject || '无主题', details, message.unread ? '未读' : '')
    if (message.preview) card.append(element('p', message.preview, 'richtext-body'))
    return card
  })
  showResults(rows, '这个范围内没有邮件。', result.observedAt)
}

function showTimetable(result) {
  const rows = (result.classes || []).map(item => row(item.title, [timeRange(item), item.location].filter(Boolean).join(' · ')))
  showResults(rows, '这一天没有课程。', result.observedAt)
}

function showAvailability(result, service, date) {
  const rows = (result.slots || []).map(slot => {
    const card = row(slot.title, [timeRange(slot), slot.location].filter(Boolean).join(' · '))
    addAction(card, '预约…', () => prepare('prepare-booking', { service, date, slotId: slot.id }))
    return card
  })
  showResults(rows, '这一天没有可预约时段。', result.observedAt)
}

function showBookings(result, service) {
  const rows = (result.bookings || []).map(booking => {
    const slot = booking.slot
    const badge = booking.status === 'reserved' ? '已预约' : '已取消'
    const card = row(slot.title, [timeRange(slot, true), slot.location].filter(Boolean).join(' · '), badge)
    if (booking.status === 'reserved') {
      addAction(card, '取消预约…', () => prepare('prepare-cancellation', { service, bookingId: booking.id }))
    }
    return card
  })
  showResults(rows, '暂无预约。')
}

function prepare(action, payload) {
  return perform(() => request(action, payload), ({ mode, intent }) => {
    preview = intent
    const cancellation = intent.operation === 'cancel'
    $('confirmation-title').textContent = cancellation ? '取消这项预约？' : '预约这个时段？'
    $('confirm-button').textContent = cancellation ? '确认取消' : '确认预约'
    $('confirmation-detail').replaceChildren(
      element('strong', intent.slot.title),
      element('p', timeRange(intent.slot, true)),
      element('p', intent.slot.location),
      element('p', mode === 'demo' ? '本地模拟' : services[intent.service].source, 'muted')
    )
    $('confirmation').showModal()
  })
}

function showReceipt({ mode, receipt }) {
  const succeeded = (receipt.operation === 'book' && receipt.status === 'reserved') ||
    (receipt.operation === 'cancel' && receipt.status === 'cancelled')
  const label = succeeded ? (receipt.operation === 'cancel' ? '预约已取消' : '预约已确认')
    : ({ pending: '处理中', failed: '操作失败', conflict: '时段冲突' }[receipt.status] || `回执状态：${receipt.status}`)
  const title = `${mode === 'demo' ? '模拟 · ' : ''}${label}`
  const details = [receipt.slot.title, timeRange(receipt.slot, true), receipt.slot.location].filter(Boolean).join(' · ')
  $('receipt').replaceChildren(
    element('strong', title), element('p', details, 'campus-row-meta'),
    element('p', `回执编号 ${receipt.id}`, 'muted')
  )
  $('receipt').hidden = false
  if (succeeded) clearResults()
}

for (const button of document.querySelectorAll('[data-service]')) {
  button.addEventListener('click', () => {
    if (busy || selected === button.dataset.service) return
    selected = button.dataset.service
    clearPrivateView()
    feedback()
    renderService()
  })
}

$('official').addEventListener('click', () => perform(
  () => invoke('open_campus_service', { service: selected }), () => feedback('已打开官方入口。')
))
$('connect').addEventListener('click', () => perform(
  () => request('connect', { service: selected, account: 'student' }), acceptStatus
))
$('disconnect').addEventListener('click', () => perform(
  () => request('disconnect', { service: selected }), acceptStatus
))

$('query-form').addEventListener('submit', event => {
  event.preventDefault()
  if (!serviceStatus()?.connected || busy) return
  const service = selected
  const date = $('date').value
  clearResults()
  if (service === 'mail') {
    perform(() => request('mail', { unreadOnly: $('unread').checked }), showMail)
  } else if (service === 'timetable') {
    perform(() => request('timetable', { date }), showTimetable)
  } else {
    perform(() => request('availability', { service, date }), result => showAvailability(result, service, date))
  }
})

$('bookings').addEventListener('click', () => {
  if (!serviceStatus()?.connected || busy) return
  const service = selected
  clearResults()
  perform(() => request('bookings', { service }), result => showBookings(result, service))
})

$('date').addEventListener('change', clearResults)
$('unread').addEventListener('change', clearResults)
$('confirmation').addEventListener('cancel', () => { preview = null })
$('confirmation-form').addEventListener('submit', event => {
  event.preventDefault()
  const intent = preview
  preview = null
  $('confirmation').close()
  if (event.submitter?.value !== 'confirm' || !intent) return
  perform(() => request('confirm', { intentId: intent.id, confirmed: true }), showReceipt)
})

$('date').value = campusDate()
perform(() => request('status'), acceptStatus, true)
window.addEventListener('focus', () => perform(() => request('status'), acceptStatus, true))
