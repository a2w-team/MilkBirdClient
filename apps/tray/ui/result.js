const statusLine = document.getElementById('status')
const card = document.getElementById('result')
const { invoke } = window.__TAURI__.core
const { listen } = window.__TAURI__.event
const appWindow = window.__TAURI__.window.getCurrentWindow()

document.getElementById('close-note').addEventListener('click', () => {
  appWindow.close().catch(error => console.error('Failed to close the note', error))
})

async function render() {
  const result = await invoke('current_result')
  card.hidden = result?.status !== 'matched'

  switch (result?.status) {
    case 'matched': {
      const announcement = result.announcement
      statusLine.textContent = announcement.campus + ' · ' + announcement.topic
      document.getElementById('title').textContent = announcement.title
      document.getElementById('summary').textContent = announcement.summary
      document.getElementById('source').textContent = announcement.source
      document.getElementById('source-url').textContent = announcement.sourceUrl
      break
    }
    case 'needs_clarification':
      statusLine.textContent = '请在 DS 中补充' + result.missingFields.map(field => ({ campus: '校区', topic: '主题' })[field] ?? field).join('、') + '。'
      break
    case 'no_match':
      statusLine.textContent = '没有符合校区和主题的有效公告。'
      break
    case 'error':
      statusLine.textContent = '查询暂时失败，请稍后在 DS 中重试。'
      break
    default:
      statusLine.textContent = '等待 DS 提交已确认的查询。'
  }
}

function showReadFailure() {
  statusLine.textContent = '暂时无法读取托盘结果。'
  card.hidden = true
}

listen('lookup-changed', () => render().catch(showReadFailure))
  .then(render)
  .catch(showReadFailure)
