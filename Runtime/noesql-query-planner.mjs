// Bounded, deterministic reference planner for a single optional equi-join.
// It does not claim native optimization or a persisted physical index.
// Plan shape depends only on source, table cardinalities and schema metadata.
// Runtime always rechecks the complete ON predicate after candidate pruning.
const SCALAR = new Set(['column','literal','param'])
const COMPARES = new Set(['==','!=','<','<=','>','>='])

export function safePredicate(e) {
  if (!e || e.k !== 'binary') return false
  if (e.op === '&&' || e.op === '||') return safePredicate(e.left) && safePredicate(e.right)
  return COMPARES.has(e.op) && SCALAR.has(e.left?.k) && SCALAR.has(e.right?.k)
}

export function baseOnly(e, table) {
  if (!e) return true
  if (e.k === 'column') return !e.name.includes('.') || e.name.startsWith(table + '.')
  if (e.k === 'binary') return baseOnly(e.left, table) && baseOnly(e.right, table)
  if (e.k === 'unary') return baseOnly(e.arg, table)
  return true
}

function equalityTerms(e, list = []) {
  if (e?.k === 'binary' && e.op === '&&') {
    equalityTerms(e.left, list)
    equalityTerms(e.right, list)
  } else if (e?.k === 'binary' && e.op === '==' && e.left?.k === 'column' && e.right?.k === 'column') list.push(e)
  return list
}

export function chooseRelationalPlan(select, leftTable, rightTable = null) {
  const leftRows = leftTable.rows.length
  const rightRows = rightTable?.rows.length ?? 0
  const pushWhere = Boolean(rightTable && select.where && safePredicate(select.where) && baseOnly(select.where,select.table))
  if (!rightTable) return Object.freeze({join:'none',leftRows,rightRows,pushWhere:false,leftKey:null,rightKey:null})
  const leftName = name => name.startsWith(select.table + '.') ? name.slice(select.table.length + 1) : !name.includes('.') ? name : null
  const rightName = name => name.startsWith(select.join.table + '.') ? name.slice(select.join.table.length + 1) : null
  let leftKey = null, rightKey = null
  // Never lift arithmetic or function calls out of ON; they can throw and
  // pruning their evaluation could change the reference evaluator semantics.
  if (safePredicate(select.join.on)) {
    for (const term of equalityTerms(select.join.on)) {
      const a=term.left.name,b=term.right.name
      if(leftName(a) && rightName(b)){leftKey=leftName(a);rightKey=rightName(b);break}
      if(leftName(b) && rightName(a)){leftKey=leftName(b);rightKey=rightName(a);break}
    }
  }
  // With tiny relations an allocation-free nested loop beats building a hash.
  // For larger products, a hash limits comparisons to equal-key candidates.
  const join = leftKey!==null && leftRows*rightRows>=64 ? 'hash' : 'nested'
  return Object.freeze({join,leftRows,rightRows,pushWhere,leftKey,rightKey})
}