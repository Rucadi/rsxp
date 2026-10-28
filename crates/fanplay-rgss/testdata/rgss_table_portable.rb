# SPDX-License-Identifier: GPL-2.0-or-later
def check(value, message)
  raise message unless value
end
table=Table.new(2,2)
table[1,1]=-1234
table[0,0]=65535
check(table[0,0]==-1, 'Table value is not signed 16-bit')
check(table[-1,0].nil?, 'out-of-range Table reads must return nil')
table.resize(3,3,2)
check(table[1,1,0]==-1234 && table[2,2,1]==0, 'Table resize lost data')
copy=Marshal.load(Marshal.dump(table))
cloned=table.clone
table[1,1,0]=42
GC.start
check(copy[1,1,0]==-1234 && cloned[1,1,0]==-1234, 'Table copies share storage')
100.times { temp=Table.new(3); temp[1]=12; check(Marshal.load(Marshal.dump(temp))[1]==12, 'Table GC roundtrip') }
Table.allocate.resize(2)
begin
  Table._load('invalid')
  raise 'bad Table payload was accepted'
rescue ArgumentError
end
puts 'TABLE_PORTABLE_OK'
