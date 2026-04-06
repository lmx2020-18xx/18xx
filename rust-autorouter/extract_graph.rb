#!/usr/bin/env ruby
# frozen_string_literal: true

# Extract a game's graph state as JSON for the Rust autorouter.
# Usage: ruby rust-autorouter/extract_graph.rb spec/data/autorouter/34.json > rust-autorouter/tests/fixtures/game_34_graph.json

require 'json'

# Silence all logging during game loading by redirecting stdout before requiring engine
$stdout_saved = $stdout.dup
$stdout = File.open(File::NULL, 'w')

require_relative '../lib/engine'

file = ARGV[0] || 'spec/data/autorouter/34.json'
data = JSON.parse(File.read(file))
game = Engine::Game.load(data)

corporation = game.current_entity
trains = game.route_trains(corporation).sort_by(&:price)
graph = game.graph_for_entity(corporation)

# Compute connected graph
connected_nodes = graph.connected_nodes(corporation)
connected_paths = graph.connected_paths(corporation)

# Build node index map
node_map = {}
nodes_json = []
connected_nodes.keys.each_with_index do |node, idx|
  node_map[node] = idx

  # Get revenue for all phases
  revenue = {}
  %w[yellow green brown gray diesel].each do |color|
    rev = begin
      node.route_base_revenue(Engine::Phase.new(color, on: '', train_limit: {}, tiles: [color.to_sym]), trains.first)
    rescue StandardError
      nil
    end
    revenue[color] = rev if rev && rev > 0
  end

  # Simple revenue: just use the current phase
  current_rev = begin
    node.route_revenue(game.phase, trains.first)
  rescue StandardError
    0
  end
  revenue[game.phase.tiles.last.to_s] = current_rev if current_rev > 0

  node_type = if node.city?
                'city'
              elsif node.town?
                'town'
              elsif node.offboard?
                'offboard'
              else
                'junction'
              end

  tokens = if node.respond_to?(:tokens)
             node.tokens.map { |t| t&.corporation&.id }
           else
             []
           end

  slots = node.respond_to?(:slots) ? node.slots : 0
  groups = node.respond_to?(:groups) ? node.groups : []
  visit_cost = node.respond_to?(:visit_cost) ? node.visit_cost : 1
  extra_tokens = if node.respond_to?(:extra_tokens)
                   node.extra_tokens.map { |t| t&.corporation&.id }.compact
                 else
                   []
                 end

  nodes_json << {
    id: idx,
    hex_id: node.hex.id,
    index: node.index || 0,
    type: node_type,
    revenue: revenue,
    slots: slots,
    tokens: tokens,
    extra_tokens: extra_tokens,
    groups: groups,
    visit_cost: visit_cost,
    is_offboard: node.offboard? || false,
  }
end

# Collect ALL paths on connected hexes (not just connected_paths)
# Ruby's walk accesses hex.paths[edge] which includes ALL tile paths,
# so Rust needs them too for parity.
connected_hexes = connected_paths.keys.map(&:hex).uniq
all_hex_paths = connected_hexes.flat_map { |hex| hex.tile.paths }.uniq

# Build path data
paths_json = []
path_map = {}
path_idx = 0
all_hex_paths.each do |path|
  path_map[path] = path_idx

  # Determine endpoints
  a_endpoint = if path.a.respond_to?(:edge?) && path.a.edge?
                { type: 'edge', num: path.a.num }
              elsif path.a.respond_to?(:junction?) && path.a.junction?
                { type: 'junction', index: 0 } # will be resolved later
              elsif node_map[path.a]
                { type: 'node', index: node_map[path.a] }
              else
                { type: 'edge', num: 0 }
              end

  b_endpoint = if path.b.respond_to?(:edge?) && path.b.edge?
                { type: 'edge', num: path.b.num }
              elsif path.b.respond_to?(:junction?) && path.b.junction?
                { type: 'junction', index: 0 }
              elsif node_map[path.b]
                { type: 'node', index: node_map[path.b] }
              else
                { type: 'edge', num: 0 }
              end

  # Lanes
  lanes = nil
  if path.exit_lanes && !path.exit_lanes.empty?
    a_lane = path.exit_lanes.values.first || [1, 0]
    b_lane = path.exit_lanes.values.last || [1, 0]
    lanes = [a_lane, b_lane]
  else
    lanes = [[1, 0], [1, 0]]
  end

  # Junction
  junction_id = nil
  if path.junction
    # Track junctions
    junction_id = path.junction.object_id
  end

  paths_json << {
    id: path_idx,
    hex_id: path.hex.id,
    a: a_endpoint,
    b: b_endpoint,
    track: (path.track || :broad).to_s,
    lanes: lanes,
    terminal: path.terminal? || false,
    ignore: path.ignore? || false,
    junction_id: junction_id,
  }

  path_idx += 1
end

# Build hex data
hexes_json = {}
seen_hexes = Set.new
all_hex_paths.each do |path|
  hex = path.hex
  next if seen_hexes.include?(hex.id)

  seen_hexes << hex.id

  neighbors = {}
  hex.neighbors.each do |edge_num, neighbor|
    next unless neighbor

    neighbors[edge_num.to_s] = neighbor.id
  end

  hexes_json[hex.id] = { neighbors: neighbors }
end

# Converging exits
converging_exits = {}
seen_hexes.each do |hex_id|
  hex = game.hex_by_id(hex_id)
  next unless hex&.tile

  exits = {}
  hex.tile.paths.group_by { |p| p.edges.map(&:num) }.each do |edges, paths_group|
    edges.each do |e|
      count = hex.tile.paths.count { |p| p.edges.any? { |pe| pe.num == e } }
      exits[e] = true if count > 1
    end
  end

  converging_exits[hex_id] = exits.keys unless exits.empty?
end

# Build junction data
junctions_json = []
junction_map = {}
all_hex_paths.each do |path|
  next unless path.junction

  jid = path.junction.object_id
  next if junction_map[jid]

  junction_paths = path.junction.paths.filter_map { |jp| path_map[jp] }
  junction_idx = junctions_json.size
  junction_map[jid] = junction_idx

  junctions_json << {
    id: junction_idx,
    hex_id: path.hex.id,
    path_ids: junction_paths,
  }
end

# Update junction_id references in paths
paths_json.each do |p|
  if p[:junction_id]
    p[:junction_id] = junction_map[p[:junction_id]]
  end
end

# Build trains data
trains_json = trains.each_with_index.map do |train, idx|
  dist = if train.distance.is_a?(Numeric)
           train.distance
         else
           train.distance.map do |d|
             {
               nodes: d['nodes'] || d[:nodes],
               pay: d['pay'] || d[:pay],
               visit: d['visit'] || d[:visit],
               multiplier: d['multiplier'] || d[:multiplier],
             }
           end
         end

  {
    id: "#{train.name}-#{idx}",
    name: train.name,
    distance: dist,
    price: train.price || 0,
    local: train.local? || false,
  }
end

# Build start_nodes in priority order (same as auto_router.rb)
start_nodes = connected_nodes.keys.sort_by do |node|
  revenue = trains.map { |train| node.route_revenue(game.phase, train) }.max
  [
    node.tokened_by?(corporation) ? 0 : 1,
    node.offboard? ? 0 : 1,
    -revenue,
  ]
end.map { |node| node_map[node] }

walk_corporation = graph.no_blocking? ? nil : corporation

# Build bonuses (game-specific revenue adjustments pre-evaluated for the current corporation)
bonuses_json = []
if game.class.title == '1846'
  # Private company bonuses
  [
    ['BT', 20, nil],
    ['MPC', 30, nil],
    ['SC', 20, 'port'],
  ].each do |company_id, base_amount, icon|
    next unless corporation.assigned?(company_id)

    matched = nodes_json.select { |n| game.hex_by_id(n[:hex_id])&.assigned?(company_id) }
    next if matched.empty?

    amount = base_amount
    if icon
      hex = game.hex_by_id(matched.first[:hex_id])
      icon_count = hex.tile.icons.count { |i| i.name == icon }
      amount = base_amount * icon_count
    end
    bonuses_json << { type: 'hex_route', nodes: matched.map { |n| n[:id] }, amount: amount }
  end

  # East/West bonus
  east_nodes = []
  west_nodes = []
  east_amount = 0
  west_amount = 0
  nodes_json.each do |n|
    hex = game.hex_by_id(n[:hex_id])
    node_obj = hex&.tile&.nodes&.find { |nn| (nn.index || 0) == n[:index] }
    next unless node_obj

    if node_obj.groups.include?('E')
      east_nodes << n[:id]
      amt = node_obj.tile.icons.sum { |ic| ic.name.to_i }
      east_amount = amt if amt > east_amount
    end
    if node_obj.tile.label&.to_s == 'W'
      west_nodes << n[:id]
      amt = node_obj.tile.icons.sum { |ic| ic.name.to_i }
      west_amount = amt if amt > west_amount
    end
  end
  if east_nodes.any? && west_nodes.any?
    bonuses_json << {
      type: 'east_west', east_nodes: east_nodes, west_nodes: west_nodes,
      east_amount: east_amount, west_amount: west_amount,
    }
  end

  # Mail Contract
  if game.respond_to?(:mail_contract)
    mc = game.mail_contract
    bonuses_json << { type: 'per_stop', amount: 10 } if mc && corporation.companies.include?(mc)
  end
elsif game.class.title == '1870'
  # SCC cattle bonus: +10 per route if corp is assigned SCC and route touches an SCC hex
  if corporation.assigned?('SCC')
    scc_nodes = nodes_json.select { |n| game.hex_by_id(n[:hex_id])&.assigned?('SCC') }
                          .map { |n| n[:id] }
    bonuses_json << { type: 'hex_route', nodes: scc_nodes, amount: 10 } unless scc_nodes.empty?
  end

  # GSC closed port: +20 per route if corp is assigned GSCᶜ and route touches GSCᶜ hex
  if corporation.assigned?('GSCᶜ')
    gsc_closed_nodes = nodes_json.select { |n| game.hex_by_id(n[:hex_id])&.assigned?('GSCᶜ') }
                                 .map { |n| n[:id] }
    bonuses_json << { type: 'hex_route', nodes: gsc_closed_nodes, amount: 20 } unless gsc_closed_nodes.empty?
  end

  # GSC open port: +20 if corp owns GSC, +10 otherwise, for route touching GSC hex
  gsc_open_nodes = nodes_json.select { |n| game.hex_by_id(n[:hex_id])&.assigned?('GSC') }
                             .map { |n| n[:id] }
  unless gsc_open_nodes.empty?
    gsc_amount = corporation.assigned?('GSC') ? 20 : 10
    bonuses_json << { type: 'hex_route', nodes: gsc_open_nodes, amount: gsc_amount }
  end

  # Destination bonus: double endpoint revenue if corp's token is on its destination hex
  # Destination tokens live in extra_tokens, not regular token slots
  if game.respond_to?(:destination_hex)
    dest_hex = game.destination_hex(corporation)
    if dest_hex
      dest_node = nodes_json.find { |n| n[:hex_id] == dest_hex.id }
      if dest_node
        has_token = dest_node[:tokens].include?(corporation.id) ||
                    dest_node[:extra_tokens].include?(corporation.id)
        bonuses_json << { type: 'destination', node: dest_node[:id] } if has_token
      end
    end
  end
end

output = {
  corporation_id: corporation.id,
  current_phase: game.phase.tiles.last.to_s,
  no_blocking: graph.no_blocking? || false,
  hexes: hexes_json,
  nodes: nodes_json,
  paths: paths_json,
  junctions: junctions_json,
  converging_exits: converging_exits,
  trains: trains_json,
  start_nodes: start_nodes,
  static_routes: [],
  bonuses: bonuses_json,
  config: {
    path_timeout_ms: 30_000,
    route_timeout_ms: 10_000,
    route_limit: 10_000,
    train_autoroute_groups: nil,
  },
}

# Restore stdout for JSON output
$stdout = $stdout_saved
puts JSON.pretty_generate(output)
