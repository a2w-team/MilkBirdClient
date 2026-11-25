'use strict'

const { invoke } = window.__TAURI__.core
const { listen } = window.__TAURI__.event
const appWindow = window.__TAURI__.window.getCurrentWindow()
const $ = id => document.getElementById(id)

let bootstrap
let view = 'demands' // demands | detail | room
let selectedDemandId = null
let selectedRoomId = null
let revisionsOpen = false
let cache = {
  demands: [],
  activeCount: 0,
  maxActive: 3,
  matches: [],
  rooms: [],
  responsesByDemand: new Map(),
  revisionsByDemand: new Map()
}
const revisionRequests = new Map()
let chat = null
let pendingRoomSends = new Set()
let syncRevision = 0
let loadRevision = 0
let pendingEvents = []
let pendingRefresh = false

function node(tag, text, className) {
  const el = document.createElement(tag)
  if (text !== undefined && text !== null) el.textContent = text
  if (className) el.className = className
  return el
}

function feedback(message = '', error = false) {
  const line = $('feedback')
  line.textContent = message
  line.hidden = !message
  line.classList.toggle('error', error)
}

function report(error) {
  const codeMap = {
    active_demand_limit_reached: '最多同时挂起 3 个需求',
    room_expired: '群聊已满 24 小时，投票保留后可继续发言',
    demand_not_editable: '当前需求不可修改',
    invalid_campus_email: '该校园邮箱无法使用，请检查邮箱名和后缀',
    invalid_or_expired_code: '验证码无效或已过期，请检查或重新获取'
  }
  feedback(codeMap[error?.code] || error?.message || '操作失败，请重试', true)
}

async function act(button, work) {
  if (button.disabled) return
  button.disabled = true
  feedback()
  try {
    await work()
  } catch (error) {
    report(error)
  } finally {
    button.disabled = false
  }
}

function makeButton(label, work, className = 'btn') {
  const btn = node('button', label, className)
  btn.type = 'button'
  btn.addEventListener('click', event => {
    event.stopPropagation()
    act(btn, work)
  })
  return btn
}

function memberName(member) {
  return member?.displayName || member?.email || '同学'
}

function formatReason(raw) {
  const map = { same_category_and_campus: '同校区 · 时间相容' }
  return map[raw] || raw || ''
}

function safeExternalUrl(raw) {
  if (typeof raw !== 'string') return null
  try {
    const u = new URL(raw)
    return (u.protocol === 'https:' || u.protocol === 'http:') ? u.href : null
  } catch {
    return null
  }
}

function remainingLabel(iso) {
  if (!iso) return ''
  const diffMs = new Date(iso).getTime() - Date.now()
  if (Number.isNaN(diffMs)) return ''
  if (diffMs <= 0) return '已到期'
  const hours = Math.ceil(diffMs / 3600000)
  return hours >= 24 ? `${Math.round(hours / 24)}天` : `${hours}h`
}

function shortTime(iso) {
  if (!iso) return ''
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return ''
  return d.toLocaleString([], { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' })
}

function demandMetaLine(demand) {
  const venue = [demand.campus, demand.location].filter(Boolean).join(' · ')
  const pMin = demand.partySize?.min
  const pMax = demand.partySize?.max
  const party = pMin && pMin > 1
    ? (pMax && pMax !== pMin ? `${pMin}–${pMax}人` : `${pMin}人`)
    : ''
  const start = demand.timeWindow?.start ? shortTime(demand.timeWindow.start) : ''
  return [venue, start, party].filter(Boolean).join(' · ')
}

function matchesForDemand(demandId) {
  return cache.matches.filter(m =>
    m.myDemandId === demandId ||
    (m.participants || []).some(p => p.demandId === demandId)
  )
}

function pendingMatchForDemand(demandId) {
  const matches = matchesForDemand(demandId).filter(match => match.status === 'pending')
  return matches.find(match => match.myDecision === 'pending') || matches[0] || null
}

function roomForDemand(demandId) {
  const direct = cache.rooms.find(r => r.demandId === demandId)
  if (direct) return direct
  for (const { roomId } of matchesForDemand(demandId)) {
    const room = roomId && cache.rooms.find(item => item.id === roomId)
    if (room) return room
  }
  return null
}

function unreadForDemand(demandId) {
  const entry = bootstrap?.unreadByDemand?.[demandId] || {}
  const responseCount = Number(entry.responseCount || 0)
  const room = roomForDemand(demandId)
  const roomCount = room
    ? Math.max(Number(entry.roomMessageCount || 0), Number(bootstrap?.unreadByRoom?.[room.id] || 0))
    : Number(entry.roomMessageCount || 0)
  return { responseCount, roomCount }
}

function closeAccountMenu() {
  $('account-menu').hidden = true
  $('account-trigger').setAttribute('aria-expanded', 'false')
}

function renderShell() {
  const { loggedIn, member, notice } = bootstrap
  $('login').hidden = loggedIn
  $('app').hidden = !loggedIn
  $('account-wrap').hidden = !loggedIn
  $('slot-dots').hidden = !loggedIn

  if (!loggedIn) {
    closeAccountMenu()
    $('notice').hidden = true
    return
  }

  const display = memberName(member)
  $('avatar-letter').textContent = display.slice(0, 1).toUpperCase()
  $('identity').textContent = display
  $('menu-name').textContent = display
  $('menu-email').textContent = member?.email || ''

  const dots = []
  for (let i = 0; i < cache.maxActive; i++) {
    dots.push(node('span', undefined, i < cache.activeCount ? 'slot-dot filled' : 'slot-dot'))
  }
  $('slot-dots').replaceChildren(...dots)
  $('slot-dots').title = `进行中 ${cache.activeCount}/${cache.maxActive}`

  $('notice').hidden = !notice
  $('notice-title').textContent = notice?.title || ''
  $('notice-body').textContent = formatReason(notice?.body)

  $('view-demands').hidden = view !== 'demands'
  $('view-detail').hidden = view !== 'detail'
  $('view-room').hidden = view !== 'room'
}

function setView(next, demandId = null, roomId = null) {
  if (next !== 'detail' || demandId !== selectedDemandId) revisionsOpen = false
  view = next
  if (demandId !== undefined) selectedDemandId = demandId
  if (roomId !== undefined) selectedRoomId = roomId
  if (view === 'room' && selectedRoomId) {
    if (!selectedDemandId) {
      const r = cache.rooms.find(item => item.id === selectedRoomId)
      if (r?.demandId) selectedDemandId = r.demandId
    }
    selectChatRoom(selectedRoomId)
    renderMessageComposer()
  } else if (view !== 'room') {
    chat = null
  }
  renderShell()
}

function rememberView() {
  if (!bootstrap?.loggedIn) return Promise.resolve()
  return invoke('remember_view', {
    navigationId: bootstrap.navigationId,
    view,
    demandId: selectedDemandId || undefined,
    roomId: view === 'room' ? (selectedRoomId || undefined) : undefined
  }).catch(() => {})
}

async function openDemandDetail(demandId) {
  const demandCache = cache
  const isCurrent = () => cache === demandCache && view === 'detail' && selectedDemandId === demandId
  setView('detail', demandId, null)
  await rememberView()
  if (!isCurrent()) return
  await invoke('mark_demand_read', { demandId }).catch(() => {})
  if (!isCurrent()) return
  await Promise.all([loadDemandResponses(demandId), loadDemandRevisions(demandId)])
  if (isCurrent()) renderDetail()
}

async function openChatRoom(roomId, demandId = null) {
  const resolvedDemand = demandId || cache.rooms.find(r => r.id === roomId)?.demandId || selectedDemandId
  setView('room', resolvedDemand, roomId)
  await rememberView()
  await invoke('mark_room_read', { roomId, demandId: resolvedDemand || undefined }).catch(() => {})
  renderRoomHeader()
  await loadMessages()
}

function renderSubmissions(submissions) {
  const statusLabels = { pending: '待提交', withdrawing: '待撤回', failed: '提交失败',
    withdrawal_failed: '撤回失败', expired: '已过期' }
  const rows = submissions.map(item => {
    const row = node('div', undefined, 'spec-row')
    const text = node('div', undefined, 'spec-v')
    text.append(node('strong', item.title || '需求'), node('div', statusLabels[item.status], 'muted'))
    if (item.error) text.append(node('div', item.error, 'muted'))
    row.append(text)
    if (item.status !== 'withdrawing') {
      const label = item.status === 'withdrawal_failed' ? '重试撤回' : '撤回'
      row.append(makeButton(label, async () => {
        await invoke('withdraw_submission', { idempotencyKey: item.idempotencyKey })
        await sync()
      }, 'btn-quiet danger-text'))
    }
    return row
  })
  $('outbox-items').replaceChildren(...rows)
  $('outbox-wrap').hidden = rows.length === 0
}

async function refreshSubmissions(revision, memberId) {
  try {
    const queue = await invoke('list_submissions')
    if (revision === syncRevision && queue.memberId === memberId) {
      renderSubmissions(queue.submissions)
    }
  } catch (error) {
    if (revision === syncRevision && !['not_logged_in', 'session_changed'].includes(error?.code)) report(error)
  }
}

function renderDemandsHome() {
  $('slot-count').textContent = `${cache.activeCount}/${cache.maxActive}`
  const openBtn = $('open-new-sheet')
  openBtn.disabled = cache.activeCount >= cache.maxActive

  const active = []
  const history = []
  for (const demand of cache.demands) {
    const room = roomForDemand(demand.id)
    const isFinished = ['completed', 'cancelled', 'canceled', 'expired'].includes(demand.status) && !room
    if (isFinished) history.push(demand)
    else active.push(demand)
  }

  const buildCard = demand => {
    const match = pendingMatchForDemand(demand.id)
    const room = roomForDemand(demand.id)
    const unread = unreadForDemand(demand.id)
    const hasAttention = unread.responseCount > 0 || unread.roomCount > 0 || match?.myDecision === 'pending'

    const card = node('button', undefined, hasAttention ? 'demand-card has-attention' : 'demand-card')
    card.type = 'button'
    card.addEventListener('click', () => openDemandDetail(demand.id).catch(report))

    const top = node('div', undefined, 'demand-top')
    top.append(node('span', demand.title, 'demand-title'))

    const right = node('div', undefined, 'demand-right')
    const ver = Number(demand.version || 1)
    if (ver > 1) right.append(node('span', `v${ver}`, 'pill ver'))

    let statusText = '寻找中'
    let statusCls = 'pill'
    if (room) {
      statusText = room.retentionStatus === 'retained' ? '长期群' : '群聊中'
      statusCls = 'pill ok'
    } else if (match) {
      statusText = match.myDecision === 'accept' ? '待确认' : '有邀请'
      statusCls = 'pill accent'
    } else if (demand.status === 'completed') {
      statusText = '已搞定'
    } else if (demand.status === 'cancelled' || demand.status === 'canceled') {
      statusText = '已撤回'
    } else if (demand.status === 'expired') {
      statusText = '已到期'
    }
    right.append(node('span', statusText, statusCls))
    top.append(right)
    card.append(top)

    const meta = demandMetaLine(demand)
    if (meta) card.append(node('span', meta, 'demand-meta'))

    const pills = node('div', undefined, 'demand-pills')
    if (unread.responseCount > 0) {
      const chip = node('span', '回应', 'pill accent')
      chip.append(node('span', String(unread.responseCount), 'unread-dot'))
      pills.append(chip)
    }
    if (room) {
      const label = room.retentionStatus === 'retained'
        ? `群聊 · ${room.members?.length || 0}人`
        : `群聊 · ${remainingLabel(room.expiresAt)}`
      const chip = node('span', label, 'pill ok')
      if (unread.roomCount > 0) chip.append(node('span', String(unread.roomCount), 'unread-dot'))
      pills.append(chip)
    } else if (match) {
      const accepted = (match.participants || []).filter(p => p.decision === 'accept').length
      const total = (match.participants || []).length
      pills.append(node('span', `${accepted}/${total} 已同意`, 'pill accent'))
    }
    if (pills.childElementCount) card.append(pills)
    return card
  }

  const activeBox = $('active-demands')
  if (active.length) {
    activeBox.replaceChildren(...active.map(buildCard))
  } else {
    activeBox.replaceChildren(node('div', '暂无进行中需求 · 在 AI 里输入 /a2w 发起', 'empty-box'))
  }

  $('history-wrap').hidden = !history.length
  $('history-summary').textContent = `已结束 (${history.length})`
  $('history-demands').replaceChildren(...history.map(buildCard))
}

function buildHtmlCardFrame(rawHtml) {
  const frame = document.createElement('iframe')
  frame.className = 'html-card-frame'
  frame.setAttribute('sandbox', '')
  frame.setAttribute('referrerpolicy', 'no-referrer')
  frame.title = '回应卡片'
  frame.srcdoc =
    '<!doctype html><html lang="zh-CN"><head><meta charset="utf-8">' +
    '<style>html,body{margin:0;padding:0;max-width:100%;overflow-x:hidden;box-sizing:border-box;}' +
    'body{padding:10px 12px;font:13px/1.5 -apple-system,"PingFang SC",sans-serif;color:#111827;background:#ffffff;}' +
    '*{box-sizing:border-box;max-width:100%;}img{border-radius:8px;height:auto;}a{color:#2563eb;}</style></head><body>' +
    rawHtml + '</body></html>'
  return frame
}

function renderRevisions(demandId) {
  if (view !== 'detail' || selectedDemandId !== demandId) return
  const panel = $('revision-panel')
  if (!revisionsOpen) {
    panel.hidden = true
    return
  }
  const list = cache.revisionsByDemand.get(demandId) || []
  panel.hidden = false
  if (!list.length) {
    panel.replaceChildren(node('span', '暂无变更记录', 'muted'))
    return
  }
  panel.replaceChildren(...list.map(rev => {
    const item = node('div', undefined, 'rev-item')
    const top = node('div', undefined, 'rev-top')
    top.append(node('span', `v${rev.version}`, 'pill ver'), node('span', shortTime(rev.createdAt), 'muted'))
    item.append(top)
    item.append(node('div', rev.changeNote || rev.title, 'rev-note'))
    const snap = demandMetaLine(rev)
    if (snap) item.append(node('div', `${rev.title} · ${snap}`, 'rev-snap'))
    return item
  }))
}

function renderDetail() {
  const demand = cache.demands.find(d => d.id === selectedDemandId)
  if (!demand) {
    setView('demands', null, null)
    renderDemandsHome()
    return
  }

  const actions = $('detail-actions')
  actions.replaceChildren()
  if (demand.status === 'open' || demand.status === 'matched') {
    actions.append(makeButton('已搞定', async () => {
      await invoke('complete_demand', { demandId: demand.id })
      await refreshAll()
      setView('demands', null, null)
      renderDemandsHome()
    }, 'btn compact'))
  }
  if (demand.status === 'open') {
    actions.append(makeButton('撤回', async () => {
      await invoke('cancel_demand', { demandId: demand.id })
      await refreshAll()
      setView('demands', null, null)
      renderDemandsHome()
    }, 'btn-quiet danger-text'))
  }

  const header = $('detail-header')
  header.replaceChildren(node('h2', demand.title))
  const metaRow = node('div', undefined, 'detail-meta-row')

  const ver = Number(demand.version || 1)
  const verBtn = node('button', `v${ver} ▾`, 'ver-btn')
  verBtn.type = 'button'
  verBtn.setAttribute('aria-expanded', String(revisionsOpen))
  verBtn.title = '查看变更历史'
  verBtn.addEventListener('click', async () => {
    revisionsOpen = !revisionsOpen
    verBtn.setAttribute('aria-expanded', String(revisionsOpen))
    if (!revisionsOpen) {
      renderRevisions(demand.id)
      return
    }
    try {
      if (await loadDemandRevisions(demand.id)) renderRevisions(demand.id)
    } catch (error) {
      if (view === 'detail' && selectedDemandId === demand.id && revisionsOpen) report(error)
    }
  })
  metaRow.append(verBtn)

  const statusMap = { open: '寻找中', matched: '已成团', completed: '已搞定', cancelled: '已撤回', expired: '已到期' }
  metaRow.append(node('span', statusMap[demand.status] || demand.status, demand.status === 'matched' ? 'pill ok' : 'pill'))
  const metaLine = demandMetaLine(demand)
  if (metaLine) metaRow.append(node('span', metaLine, 'muted'))
  header.append(metaRow)

  renderRevisions(demand.id)

  const specRows = $('detail-spec')
  const fields = [
    ['地点', [demand.campus, demand.location].filter(Boolean).join(' · ')],
    ['摘要', demand.summary],
    ['人数', demand.partySize && demand.partySize.min > 1
      ? `${demand.partySize.min}${demand.partySize.max && demand.partySize.max !== demand.partySize.min ? '–' + demand.partySize.max : ''}人`
      : ''],
    ['截止', remainingLabel(demand.expiresAt)]
  ].filter(([, v]) => Boolean(v))
  if (demand.preferences && typeof demand.preferences === 'object') {
    for (const [k, v] of Object.entries(demand.preferences)) {
      if (v !== undefined && v !== null && String(v).trim()) {
        fields.push([k, Array.isArray(v) ? v.join('、') : String(v)])
      }
    }
  }
  $('detail-spec-wrap').hidden = !fields.length
  specRows.replaceChildren(...fields.map(([k, v]) => {
    const row = node('div', undefined, 'spec-row')
    row.append(node('span', k, 'spec-k'), node('span', v, 'spec-v'))
    return row
  }))

  const list = $('detail-responses')
  const items = []
  const room = roomForDemand(demand.id)
  const match = pendingMatchForDemand(demand.id)
  const responses = cache.responsesByDemand.get(demand.id) || []
  const unread = unreadForDemand(demand.id)

  // 1. Group Chat Entry
  if (room) {
    const card = node('article', undefined, 'card room-entry-card')
    const kicker = node('div', undefined, 'card-kicker')
    kicker.append(
      node('strong', `群聊 · ${room.members?.length || 0}人`),
      node('span', room.retentionStatus === 'retained' ? '长期群' : remainingLabel(room.expiresAt))
    )
    card.append(kicker)
    if (room.lastMessage?.body) {
      card.append(node('p', `${room.lastMessage.senderName || '同学'}：${room.lastMessage.body}`, 'muted'))
    }
    const cardActions = node('div', undefined, 'card-actions')
    const enterBtn = makeButton('进入群聊', () => openChatRoom(room.id, demand.id), 'btn primary compact')
    if (unread.roomCount > 0) enterBtn.append(node('span', String(unread.roomCount), 'unread-dot'))
    cardActions.append(enterBtn)
    card.append(cardActions)
    items.push(card)
  }

  // 2. Group Invite
  if (match) {
    const card = node('article', undefined, 'card invite-card')
    const participants = match.participants || []
    const others = participants.filter(p => p.memberId !== bootstrap.member.id)
    const kicker = node('div', undefined, 'card-kicker')
    kicker.append(node('strong', `${participants.length}人成团邀请`), node('span', formatReason(match.reason)))
    card.append(kicker)
    card.append(node('div', others[0]?.demandTitle || demand.title, 'card-title'))

    const chips = node('div', undefined, 'participants')
    const decMap = { accept: '✓', decline: '×', pending: '…' }
    for (const p of participants) {
      const own = p.memberId === bootstrap.member.id
      const name = own ? '你' : memberName(p)
      const cls = p.decision === 'accept' ? 'pill ok' : 'pill'
      chips.append(node('span', `${name} ${decMap[p.decision] || '…'}`, cls))
    }
    card.append(chips)

    const cardActions = node('div', undefined, 'card-actions')
    if (match.myDecision === 'pending') {
      cardActions.append(
        makeButton('同意进群', async () => {
          const result = await invoke('respond_match', { matchId: match.id, decision: 'accept' })
          await refreshAll()
          if (result?.room?.id) await openChatRoom(result.room.id, demand.id)
          else renderDetail()
        }, 'btn primary compact'),
        makeButton('跳过', async () => {
          await invoke('respond_match', { matchId: match.id, decision: 'decline' })
          await refreshAll()
          renderDetail()
        }, 'btn-quiet')
      )
    } else if (match.myDecision === 'accept') {
      const waitCount = participants.filter(p => p.decision !== 'accept').length
      cardActions.append(node('span', `等 ${waitCount} 人确认`, 'pill ok'))
    }
    card.append(cardActions)
    items.push(card)
  }

  // 3. Heterogeneous Backend Responses
  for (const resp of responses) {
    const card = node('article', undefined, 'card')
    const kicker = node('div', undefined, 'card-kicker')
    kicker.append(node('strong', resp.source || '推荐'), node('span', shortTime(resp.createdAt)))
    card.append(kicker)

    card.append(node('div', resp.title, 'card-title'))
    if (resp.why) card.append(node('div', resp.why, 'why-box'))
    if (resp.kind === 'html_card' && resp.html) {
      card.append(buildHtmlCardFrame(resp.html))
    } else if (resp.content) {
      card.append(node('div', resp.content, 'richtext-body'))
    } else if (resp.summary) {
      card.append(node('p', resp.summary, 'muted'))
    }

    const cardActions = node('div', undefined, 'card-actions')
    const href = safeExternalUrl(resp.url)
    if (href) {
      const link = node('a', '原帖 ↗', 'btn primary compact')
      link.href = href
      link.target = '_blank'
      link.rel = 'noopener noreferrer'
      cardActions.append(link)
    }
    if (!resp.feedback) {
      cardActions.append(
        makeButton('有用', async () => {
          await invoke('feedback_response', { responseId: resp.id, feedback: 'useful' })
          await loadDemandResponses(demand.id)
          renderDetail()
        }, 'btn compact'),
        makeButton('不合适', async () => {
          await invoke('feedback_response', { responseId: resp.id, feedback: 'dismissed' })
          await loadDemandResponses(demand.id)
          renderDetail()
        }, 'btn-quiet')
      )
    } else {
      cardActions.append(
        node('span', resp.feedback === 'useful' ? '有用' : '不合适', resp.feedback === 'useful' ? 'pill ok' : 'pill'),
        makeButton('撤销', async () => {
          await invoke('feedback_response', { responseId: resp.id, feedback: '' })
          await loadDemandResponses(demand.id)
          renderDetail()
        }, 'btn-quiet')
      )
    }
    card.append(cardActions)
    items.push(card)
  }

  if (items.length) {
    list.replaceChildren(...items)
  } else {
    list.replaceChildren(node('div', '等待回应中', 'empty-box'))
  }
}

function selectChatRoom(roomId) {
  if (chat?.id !== roomId) {
    chat = { id: roomId, after: 0, pending: false, running: false }
    $('messages').replaceChildren()
    $('message').value = ''
  }
}

function renderMessageComposer(room = cache.rooms.find(item => item.id === selectedRoomId)) {
  const expired = room?.retentionStatus === 'expired'
  const disabled = !room || expired
  $('message').disabled = disabled
  $('send-message').disabled = disabled || pendingRoomSends.has(room.id)
  $('message').placeholder = expired ? '已到期，投票保留后可发言' : '输入消息…'
}

function renderRoomHeader() {
  const room = cache.rooms.find(r => r.id === selectedRoomId)
  renderMessageComposer(room)
  if (!room) {
    $('room-title').textContent = '群聊'
    $('room-meta').textContent = ''
    $('retention-bar').hidden = true
    return
  }

  $('room-title').textContent = room.title || '群聊'
  const count = room.members?.length || 0
  $('room-meta').textContent = `${count}人`

  const bar = $('retention-bar')
  bar.hidden = false
  bar.className = 'retention-bar'
  bar.replaceChildren()

  const ret = room.retention || { keepCount: 0, requiredCount: Math.floor(count / 2) + 1, totalMembers: count, myVote: '' }
  const expired = room.retentionStatus === 'expired'
  const retained = room.retentionStatus === 'retained'

  if (retained) {
    bar.classList.add('retained')
    bar.append(node('span', `✓ 长期保留 (${ret.keepCount}/${ret.totalMembers})`))
  } else {
    if (expired) bar.classList.add('expired')
    const label = expired
      ? `已到期 · 保留 ${ret.keepCount}/${ret.requiredCount}`
      : `24h · 剩${remainingLabel(room.expiresAt)} · 保留 ${ret.keepCount}/${ret.requiredCount}`
    bar.append(node('span', label))

    const actions = node('div', undefined, 'retention-actions')
    actions.append(
      makeButton(ret.myVote === 'keep' ? '已投 ✓' : '保留', async () => {
        await invoke('vote_room_retention', { roomId: room.id, keep: true })
        await refreshAll()
        renderRoomHeader()
      }, ret.myVote === 'keep' ? 'btn primary compact' : 'btn compact'),
      makeButton('算了', async () => {
        await invoke('vote_room_retention', { roomId: room.id, keep: false })
        await refreshAll()
        renderRoomHeader()
      }, 'btn-quiet')
    )
    bar.append(actions)
  }
}

async function loadMessages() {
  const active = chat
  if (!active || view !== 'room') return
  active.pending = true
  if (active.running) return
  active.running = true
  try {
    while (active.pending && chat === active && view === 'room') {
      active.pending = false
      const result = await invoke('list_room_messages', { roomId: active.id, after: active.after })
      if (active !== chat || view !== 'room') return
      appendMessages(active, result.messages || [])
      await invoke('mark_room_read', { roomId: active.id, demandId: selectedDemandId || undefined }).catch(() => {})
    }
  } finally {
    active.running = false
  }
}

function appendMessages(active, items) {
  const log = $('messages')
  const follow = !active.after || log.scrollHeight - log.scrollTop - log.clientHeight < 64
  for (const item of [...items].sort((a, b) => Number(a.seq) - Number(b.seq))) {
    const seq = Number(item.seq)
    if (!Number.isSafeInteger(seq) || seq <= active.after) continue
    const own = (item.senderId || item.sender?.id) === bootstrap.member.id
    const bubble = node('article', undefined, own ? 'message own' : 'message')
    const name = own ? '你' : (item.senderName || item.senderDisplayName || memberName(item.sender))
    bubble.append(node('span', name, 'message-sender'), node('p', item.body ?? item.text ?? ''))
    log.append(bubble)
    active.after = seq
  }
  if (follow) log.scrollTop = log.scrollHeight
}

async function loadDemandResponses(demandId) {
  if (!demandId) return
  const result = await invoke('list_demand_responses', { demandId })
  cache.responsesByDemand.set(demandId, result.responses || [])
}

async function loadDemandRevisions(demandId) {
  if (!demandId) return false
  const request = invoke('list_demand_revisions', { demandId })
  revisionRequests.set(demandId, request)
  try {
    const result = await request
    if (revisionRequests.get(demandId) !== request) return false
    cache.revisionsByDemand.set(demandId, result.revisions || [])
    return true
  } catch (error) {
    if (revisionRequests.get(demandId) === request) throw error
    return false
  } finally {
    if (revisionRequests.get(demandId) === request) revisionRequests.delete(demandId)
  }
}

async function refreshAll() {
  const revision = ++loadRevision
  const [demRes, matRes, roomRes] = await Promise.all([
    invoke('list_demands'),
    invoke('list_matches'),
    invoke('list_rooms')
  ])
  if (revision !== loadRevision) return
  cache.demands = demRes.demands || []
  cache.activeCount = Number(demRes.activeCount ?? cache.demands.filter(d => d.status === 'open').length)
  cache.maxActive = Number(demRes.maxActive || 3)
  cache.matches = matRes.matches || []
  cache.rooms = roomRes.rooms || []
  if (view === 'detail' && selectedDemandId) {
    await Promise.all([loadDemandResponses(selectedDemandId), loadDemandRevisions(selectedDemandId)])
  }
  renderShell()
}

async function sync(update, force = false) {
  pendingEvents.push(...(update?.events || []))
  pendingRefresh ||= force || Boolean(update?.refreshLists)
  const revision = ++syncRevision
  const next = await invoke('get_bootstrap')
  if (revision !== syncRevision) return
  const previous = bootstrap
  const events = pendingEvents
  const refresh = pendingRefresh
  pendingEvents = []
  pendingRefresh = false
  bootstrap = next

  const accountChanged = previous?.member?.id !== next.member?.id || previous?.loggedIn !== next.loggedIn
  const navigated = previous?.navigationId !== next.navigationId
  if (accountChanged) {
    cache = { demands: [], activeCount: 0, maxActive: 3, matches: [], rooms: [], responsesByDemand: new Map(), revisionsByDemand: new Map() }
    revisionRequests.clear()
    renderSubmissions([])
    chat = null
    pendingRoomSends = new Set()
    selectedDemandId = null
    selectedRoomId = null
    revisionsOpen = false
    $('login-form').reset()
    clearLoginCode()
    feedback()
  }
  if (!next.loggedIn) {
    setView('demands', null, null)
    return
  }

  if (navigated) {
    const targetView = ['demands', 'detail', 'room'].includes(next.activeView) ? next.activeView : 'demands'
    setView(targetView, next.demandId || null, next.roomId || null)
  }

  refreshSubmissions(revision, next.member.id)

  const onlyChatMessages = !refresh && !accountChanged && !navigated && events.length > 0 && events.every(e => e.type === 'room.message')
  if (onlyChatMessages) {
    renderShell()
    if (view === 'room' && chat) await loadMessages()
    else if (view === 'demands') renderDemandsHome()
    else if (view === 'detail') renderDetail()
    return
  }

  if (refresh || accountChanged || navigated || events.length) {
    await refreshAll()
  } else {
    renderShell()
  }

  if (view === 'demands') renderDemandsHome()
  else if (view === 'detail') renderDetail()
  else if (view === 'room') {
    renderRoomHeader()
    await loadMessages()
  }
}

/* Account Popover */
$('account-trigger').addEventListener('click', event => {
  event.stopPropagation()
  const menu = $('account-menu')
  const open = menu.hidden
  menu.hidden = !open
  $('account-trigger').setAttribute('aria-expanded', String(open))
})
document.addEventListener('click', event => {
  if (!$('account-wrap').contains(event.target)) closeAccountMenu()
})

/* Navigation & Actions */
$('back-to-demands').addEventListener('click', () => {
  setView('demands', null, null)
  rememberView()
  renderDemandsHome()
})

$('back-to-detail').addEventListener('click', () => {
  if (selectedDemandId) openDemandDetail(selectedDemandId).catch(report)
  else {
    setView('demands', null, null)
    rememberView()
    renderDemandsHome()
  }
})

$('message-form').addEventListener('submit', async event => {
  event.preventDefault()
  const active = chat
  const input = $('message')
  const text = input.value.trim()
  const room = cache.rooms.find(item => item.id === active?.id)
  const roomSends = pendingRoomSends
  if (view !== 'room' || !room || !text || room.retentionStatus === 'expired' || roomSends.has(room.id)) return

  roomSends.add(room.id)
  renderMessageComposer(room)
  feedback()
  try {
    await invoke('send_room_message', { roomId: room.id, text })
    if (chat === active) {
      if (input.value.trim() === text) input.value = ''
      if (!input.disabled) input.focus()
    }
    if (roomSends === pendingRoomSends && chat?.id === room.id) await loadMessages()
  } catch (error) {
    if (chat === active) report(error)
  } finally {
    roomSends.delete(room.id)
    if (roomSends === pendingRoomSends && view === 'room') renderMessageComposer()
  }
})

$('leave-room').addEventListener('click', () => act($('leave-room'), async () => {
  const roomId = chat?.id
  if (!roomId) return
  await invoke('leave_room', { roomId })
  await refreshAll()
  if (selectedDemandId) openDemandDetail(selectedDemandId).catch(report)
  else {
    setView('demands', null, null)
    renderDemandsHome()
  }
}))

function clearLoginCode() {
  $('code').value = ''
  $('login-hint').textContent = ''
  $('request-code').textContent = '获取验证码'
}

function loginEmail() {
  const input = $('email')
  input.value = input.value.trim()
  const email = `${input.value}@${$('email-domain').value}`
  const validator = document.createElement('input')
  validator.type = 'email'
  validator.value = email
  input.setCustomValidity(input.value && !validator.validity.valid ? '请输入 @ 前的邮箱名' : '')
  return input.reportValidity() ? email.toLowerCase() : null
}

async function loginAction(button, pendingLabel, work) {
  const fields = $('login-fields')
  if (fields.disabled) return false
  const label = button.textContent
  fields.disabled = true
  button.textContent = pendingLabel
  feedback()
  try {
    await work()
    return true
  } catch (error) {
    report(error)
    return false
  } finally {
    fields.disabled = false
    button.textContent = label
  }
}

function loginAddressChanged() {
  $('email').setCustomValidity('')
  clearLoginCode()
  feedback()
}

$('email').addEventListener('input', loginAddressChanged)
$('email-domain').addEventListener('change', loginAddressChanged)

$('request-code').addEventListener('click', async () => {
  const email = loginEmail()
  if (!email) return
  const button = $('request-code')
  const sent = await loginAction(button, '发送中…', async () => {
    const { devCode } = await invoke('request_email_code', { email })
    $('code').value = ''
    $('login-hint').textContent = devCode ? `测试验证码 ${devCode}` : '验证码已发送，请查收邮箱'
  })
  if (sent) {
    button.textContent = '重新发送'
    $('code').focus()
  } else {
    button.focus()
  }
})

$('login-form').addEventListener('submit', async event => {
  event.preventDefault()
  const email = loginEmail()
  if (!email) return
  const signedIn = await loginAction($('sign-in'), '登录中…', async () => {
    await invoke('verify_email_code', {
      email,
      code: $('code').value,
      displayName: $('display-name').value
    })
    await sync(undefined, true)
  })
  if (!signedIn) $('code').focus()
})

$('logout').addEventListener('click', () => act($('logout'), async () => {
  closeAccountMenu()
  try {
    await invoke('logout')
  } finally {
    await sync()
  }
}))

$('notice').addEventListener('click', () => act($('notice'), async () => {
  if (bootstrap?.notice) await invoke('open_notification', { noticeId: bootstrap.notice.id })
}))

$('open-new-sheet').addEventListener('click', () => {
  $('copy-cmd').textContent = '复制'
  $('new-sheet').hidden = false
})
$('close-sheet').addEventListener('click', () => { $('new-sheet').hidden = true })
$('new-sheet').addEventListener('click', event => {
  if (event.target === $('new-sheet')) $('new-sheet').hidden = true
})
$('copy-cmd').addEventListener('click', () => {
  navigator.clipboard?.writeText($('sample-cmd').textContent).catch(() => {})
  $('copy-cmd').textContent = '已复制'
})

$('close-window').addEventListener('click', () => appWindow.close().catch(report))

listen('hub-event', ({ payload }) => sync(payload).catch(report))
  .then(async () => {
    const domains = await invoke('get_login_domains')
    const options = domains.map(domain => new Option('@' + domain, domain))
    options[0].defaultSelected = true
    $('email-domain').replaceChildren(...options)
    $('login-fields').disabled = false
    await sync(undefined, true)
  })
  .catch(report)
