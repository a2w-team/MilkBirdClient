const assert = require('node:assert/strict')
const { readFileSync } = require('node:fs')
const { join } = require('node:path')
const { test } = require('node:test')
const vm = require('node:vm')

const source = readFileSync(join(__dirname, '..', 'ui', 'app.js'), 'utf8')

class Element {
  constructor(tag) {
    this.tag = tag
    this.children = []
    this.textContent = ''
    this.classList = { add() {}, toggle() {} }
  }

  addEventListener() {}
  setAttribute() {}
  after(element) { this.nextElementSibling = element }
  append(...children) { this.children.push(...children) }
  replaceChildren(...children) { this.children = children }
  get childElementCount() { return this.children.length }
}

function fixture() {
  const elements = new Map()
  const handlers = new Map()
  const calls = []
  const demand = {
    id: 'd1',
    title: '二手 iPad',
    status: 'open',
    preferences: { fulfillment: 'ranked', budgetMax: 250000 },
    matchAuthorization: { status: 'active', grantId: 'g1' }
  }
  const login = { loggedIn: true, member: { id: 'owner' }, navigationId: 0 }
  const get = id => {
    if (!elements.has(id)) elements.set(id, new Element('div'))
    return elements.get(id)
  }
  const context = vm.createContext({
    URL,
    demand,
    login,
    document: {
      createElement: tag => new Element(tag),
      getElementById: get,
      addEventListener() {}
    },
    window: {
      __TAURI__: {
        core: {
          invoke(action, args) {
            calls.push(action)
            return Promise.resolve(handlers.get(action)?.(args) ?? {})
          }
        },
        event: { listen: () => new Promise(() => {}) },
        window: { getCurrentWindow: () => ({}) }
      }
    }
  })
  const run = expression => vm.runInContext(expression, context)
  run(source)
  run("bootstrap = login; cache.demands = [demand]; setView('detail', 'd1'); renderDetail()")
  return { get, run, handlers, calls, demand }
}

function descendants(element) {
  return [element, ...element.children.flatMap(descendants)]
}

test('preview renders the returned order, score, reasons and safe public source only', async () => {
  const { get, run, handlers } = fixture()
  handlers.set('rank_demand', ({ demandId }) => {
    assert.equal(demandId, 'd1')
    return {
      strategyVersion: 'rules-v1',
      candidates: [
        { kind: 'info', title: '校内闲置 iPad', url: 'https://example.test/ipad', source: '校内公告', score: 81, reasons: ['符合预算'] },
        { kind: 'peer', title: '另一项 iPad', url: 'https://example.test/private', score: 74, reasons: ['符合优先校区'] },
        { kind: 'info', title: '无效出处', url: 'javascript:alert(1)', score: 50, reasons: [] }
      ]
    }
  })
  await run("runRecommendation('rank_demand')")
  const items = descendants(get('recommendation-candidates'))
  const text = items.map(item => item.textContent).join('\n')
  assert.match(text, /rules-v1[\s\S]*校内闲置 iPad[\s\S]*得分 81[\s\S]*符合预算[\s\S]*另一项 iPad/)
  assert.deepEqual(items.filter(item => item.tag === 'a').map(item => item.href), ['https://example.test/ipad'])
})

test('a late candidate response cannot survive grant withdrawal', async () => {
  const { get, run, handlers, demand } = fixture()
  let finish
  handlers.set('rank_demand', () => new Promise(resolve => { finish = resolve }))
  const preview = run("runRecommendation('rank_demand')")
  demand.status = 'cancelled'
  demand.matchAuthorization.status = 'revoked'
  run('renderDetail()')
  finish({ candidates: [{ kind: 'peer', title: '不再授权的候选' }] })
  await preview
  assert.equal(get('detail-recommendations').hidden, true)
  assert.equal(get('recommendation-candidates').children.length, 0)
})

test('empty automatic recommendation can recheck, while manual takeover cannot', async () => {
  const { get, run, handlers, calls } = fixture()
  handlers.set('get_recommendation', () => ({
    recommendation: { status: 'no_candidate', state: 'empty', mode: 'auto', nextCheckAt: '2027-01-01T18:00:00Z' }
  }))
  handlers.set('recheck_recommendation', ({ demandId }) => {
    assert.equal(demandId, 'd1')
    return { recommendation: { status: 'waiting_operator', mode: 'manual', state: 'pending' } }
  })
  await run("runRecommendation('get_recommendation')")
  assert.match(get('recommendation-status').textContent, /暂无符合条件的候选.*下次检查/)
  assert.equal(get('recommendation-recheck').hidden, false)
  await run("runRecommendation('recheck_recommendation')")
  assert.equal(get('recommendation-status').textContent, '等待运营确认')
  assert.equal(get('recommendation-recheck').hidden, true)
  await run("runRecommendation('recheck_recommendation')")
  assert.equal(calls.filter(action => action === 'recheck_recommendation').length, 1)
})

test('previewing a demand never opts it into automatic recommendation', async () => {
  const { get, run, handlers, calls, demand } = fixture()
  delete demand.preferences.fulfillment
  handlers.set('rank_demand', () => ({ strategyVersion: 'rules-v1', candidates: [] }))
  await run("runRecommendation('rank_demand')")
  await run("runRecommendation('get_recommendation')")
  await run("runRecommendation('recheck_recommendation')")
  assert.equal(get('recommendation-refresh').hidden, true)
  assert.deepEqual(calls, ['rank_demand'])
})

test('the optional recommendation explanation renders as text when present', () => {
  const { get, run } = fixture()
  const match = {
    id: 'm1',
    myDemandId: 'd1',
    status: 'pending',
    myDecision: 'pending',
    reason: 'ranked_peer',
    why: '<strong>运营理由</strong>',
    participants: [{ memberId: 'owner', demandId: 'd1' }, { memberId: 'peer', demandTitle: '另一项 iPad' }]
  }
  run(`cache.matches = [${JSON.stringify(match)}]; renderDetail()`)
  assert.ok(descendants(get('detail-responses')).some(item => item.textContent === '<strong>运营理由</strong>'))
  run("delete cache.matches[0].why; renderDetail()")
  assert.ok(!descendants(get('detail-responses')).some(item => item.textContent.includes('运营理由')))
})
